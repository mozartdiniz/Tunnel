/// File transfer engine.
///
/// `send_file`    — outgoing: connect to peer, handshake, stream bytes.
/// `receive_file` — incoming: accept TLS conn, handshake, save bytes.
///
/// Both sides verify the SHA-256 checksum at the end.
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;

use crate::app::{AppEvent, PendingMap};
use crate::config::Config;
use crate::protocol::{self, Message, ResponseStatus, PROTOCOL_VERSION};
use crate::tls::TlsStack;

const CHUNK_SIZE: usize = 64 * 1024; // 64 KiB

// ── Outgoing ─────────────────────────────────────────────────────────────────

pub async fn send_file(
    peer_addr: SocketAddr,
    file_path: PathBuf,
    sender_name: String,
    tls: Arc<TlsStack>,
    event_tx: async_channel::Sender<AppEvent>,
) -> Result<()> {
    let transfer_id = uuid::Uuid::new_v4().to_string();

    // Open file and read metadata before connecting
    let mut file = File::open(&file_path).await?;
    let size_bytes = file.metadata().await?.len();
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    // Connect + TLS handshake
    let stream = TcpStream::connect(peer_addr).await?;
    // Use the peer IP as the server name for TOFU lookup
    let server_name = ServerName::try_from(peer_addr.ip().to_string())
        .map_err(|e| anyhow::anyhow!("Invalid server name: {e}"))?;
    let mut tls_stream = tls.connector.connect(server_name, stream).await?;

    tracing::info!("Connected to {peer_addr}, sending ASK for '{file_name}'");

    // Send ASK
    protocol::write_message(
        &mut tls_stream,
        &Message::Ask {
            version: PROTOCOL_VERSION,
            transfer_id: transfer_id.clone(),
            sender_name,
            file_name: file_name.clone(),
            size_bytes,
        },
    )
    .await?;

    // Wait for RESPONSE
    let response = protocol::read_message(&mut tls_stream).await?;
    match response {
        Message::Response {
            status: ResponseStatus::Accepted,
            ..
        } => {
            tracing::info!("Transfer accepted — streaming {size_bytes} bytes");
        }
        Message::Response {
            status: ResponseStatus::Denied,
            ..
        } => {
            bail!("Peer denied the transfer");
        }
        other => bail!("Unexpected message: {other:?}"),
    }

    // Stream file bytes with progress updates; compute checksum inline (single read)
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut bytes_sent: u64 = 0;
    let mut hasher = Sha256::new();
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        tls_stream.write_all(&buf[..n]).await?;
        bytes_sent += n as u64;
        let _ = event_tx
            .send(AppEvent::TransferProgress {
                transfer_id: transfer_id.clone(),
                bytes_done: bytes_sent,
                total_bytes: size_bytes,
            })
            .await;
    }
    tls_stream.flush().await?;

    // Send checksum computed during streaming
    protocol::write_message(
        &mut tls_stream,
        &Message::Done {
            checksum_sha256: hex::encode(hasher.finalize()),
        },
    )
    .await?;

    // Wait for receiver's checksum ACK
    let ack = protocol::read_message(&mut tls_stream).await?;
    match ack {
        Message::Response {
            status: ResponseStatus::ChecksumOk,
            ..
        } => {
            let _ = event_tx
                .send(AppEvent::TransferComplete {
                    transfer_id: transfer_id.clone(),
                    saved_to: file_path,
                })
                .await;
            tracing::info!("Transfer complete ✓");
        }
        Message::Response {
            status: ResponseStatus::ChecksumFail,
            ..
        } => bail!("Receiver reported checksum mismatch — file may be corrupted"),
        other => bail!("Unexpected ACK: {other:?}"),
    }

    Ok(())
}

// ── Incoming ──────────────────────────────────────────────────────────────────

pub async fn receive_file(
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    tls: Arc<TlsStack>,
    config: Config,
    event_tx: async_channel::Sender<AppEvent>,
    pending: PendingMap,
) -> Result<()> {
    let mut tls_stream = tls.acceptor.accept(stream).await?;
    tracing::debug!("TLS handshake complete with {peer_addr}");

    // Read ASK
    let ask = protocol::read_message(&mut tls_stream).await?;
    let (transfer_id, sender_name, file_name, size_bytes) = match ask {
        Message::Ask {
            transfer_id,
            sender_name,
            file_name,
            size_bytes,
            ..
        } => (transfer_id, sender_name, file_name, size_bytes),
        other => bail!("Expected ASK, got {other:?}"),
    };

    // Notify UI and wait for user decision
    let (decision_tx, decision_rx) = tokio::sync::oneshot::channel::<bool>();
    {
        let mut map: tokio::sync::MutexGuard<
            std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>,
        > = pending.lock().await;
        map.insert(transfer_id.clone(), decision_tx);
    }
    let _ = event_tx
        .send(AppEvent::IncomingRequest {
            transfer_id: transfer_id.clone(),
            sender_name: sender_name.clone(),
            file_name: file_name.clone(),
            size_bytes,
        })
        .await;

    let accepted = decision_rx.await.unwrap_or(false);

    // Send RESPONSE
    protocol::write_message(
        &mut tls_stream,
        &Message::Response {
            version: PROTOCOL_VERSION,
            status: if accepted {
                ResponseStatus::Accepted
            } else {
                ResponseStatus::Denied
            },
        },
    )
    .await?;

    if !accepted {
        tracing::info!("User denied transfer from {peer_addr}");
        return Ok(());
    }

    // Receive file bytes
    let safe_name = sanitize_filename(&file_name);
    let dest_path = config.download_dir.join(&safe_name);
    let dest_file = File::create(&dest_path).await?;
    let mut writer = BufWriter::new(dest_file);

    let mut received: u64 = 0;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    // We read exactly `size_bytes` bytes, then expect the Done message.
    while received < size_bytes {
        let to_read = ((size_bytes - received) as usize).min(CHUNK_SIZE);
        let n = tls_stream.read(&mut buf[..to_read]).await?;
        if n == 0 {
            bail!("Connection closed prematurely");
        }
        writer.write_all(&buf[..n]).await?;
        hasher.update(&buf[..n]);
        received += n as u64;

        let _ = event_tx
            .send(AppEvent::TransferProgress {
                transfer_id: transfer_id.clone(),
                bytes_done: received,
                total_bytes: size_bytes,
            })
            .await;
    }
    writer.flush().await?;

    // Read Done + verify checksum
    let done = protocol::read_message(&mut tls_stream).await?;
    let status = match done {
        Message::Done { checksum_sha256 } => {
            let our_checksum = hex::encode(hasher.finalize());
            if our_checksum == checksum_sha256 {
                tracing::info!("Checksum OK for '{safe_name}'");
                ResponseStatus::ChecksumOk
            } else {
                tracing::error!(
                    "Checksum FAIL: expected {checksum_sha256}, got {our_checksum}"
                );
                ResponseStatus::ChecksumFail
            }
        }
        other => bail!("Expected Done, got {other:?}"),
    };

    protocol::write_message(
        &mut tls_stream,
        &Message::Response {
            version: PROTOCOL_VERSION,
            status: status.clone(),
        },
    )
    .await?;

    if status == ResponseStatus::ChecksumOk {
        let _ = event_tx
            .send(AppEvent::TransferComplete {
                transfer_id,
                saved_to: dest_path,
            })
            .await;
    } else {
        let _ = event_tx
            .send(AppEvent::TransferError {
                transfer_id,
                message: "Checksum mismatch — file may be corrupted".into(),
            })
            .await;
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}
