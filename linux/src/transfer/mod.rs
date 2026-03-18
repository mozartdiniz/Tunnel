/// File transfer — outgoing (sender) side.
///
/// The incoming (receiver) side is handled by the axum HTTP handlers in `app/handlers/`.
///
/// Flow (LocalSend v2):
///   1. POST /api/localsend/v2/prepare-upload  — offer file list, wait for accept/deny.
///   2. POST /api/localsend/v2/upload          — stream each file's bytes.
mod helpers;
mod zip;

pub use helpers::{sanitize_filename, speed_eta, MAX_FILE_SIZE};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use anyhow::{bail, Result};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio_util::io::ReaderStream;

use crate::app::AppEvent;
use crate::inhibit::InhibitGuard;
use crate::localsend::{
    DeviceInfo, FileMetadata, PrepareUploadRequest, PrepareUploadResponse, LOCALSEND_PORT,
};
use crate::tls::TlsStack;

use helpers::{mime_guess, CHUNK_SIZE};
use zip::zip_directory;

/// Send one or more files/directories to `peer_addr`.
///
/// Directories are ZIP-compressed into a temp file before transfer.
/// All paths are sent in a single `prepare-upload` → N × `upload` round trip.
/// Emits `TransferProgress`, `TransferComplete`, and `TransferError` events.
pub async fn send_files(
    peer_addr: SocketAddr,
    paths: Vec<PathBuf>,
    sender_name: String,
    sender_fingerprint: String,
    /// The peer's announced fingerprint from UDP discovery — used as the TOFU key.
    peer_fingerprint: String,
    tls: Arc<TlsStack>,
    event_tx: async_channel::Sender<AppEvent>,
) -> Result<()> {
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let result = try_send_files(
        &transfer_id,
        peer_addr,
        paths,
        sender_name,
        sender_fingerprint,
        peer_fingerprint,
        tls,
        &event_tx,
    )
    .await;
    if let Err(ref e) = result {
        let _ = event_tx
            .send(AppEvent::TransferError {
                transfer_id,
                message: e.to_string(),
            })
            .await;
    }
    result
}

async fn try_send_files(
    transfer_id: &str,
    peer_addr: SocketAddr,
    paths: Vec<PathBuf>,
    sender_name: String,
    sender_fingerprint: String,
    peer_fingerprint: String,
    tls: Arc<TlsStack>,
    event_tx: &async_channel::Sender<AppEvent>,
) -> Result<()> {
    let _inhibit = InhibitGuard::acquire("Sending files").await;

    // ── Prepare file list (zip directories) ──────────────────────────────────
    struct FileEntry {
        source_path: PathBuf,
        display_name: String,
        size_bytes: u64,
        file_type: String,
        sha256: Option<String>,
        _temp: Option<tempfile::NamedTempFile>,
    }

    let mut entries: Vec<(String, FileEntry)> = Vec::with_capacity(paths.len());
    for path in &paths {
        let file_id = uuid::Uuid::new_v4().to_string();
        let entry = if path.is_dir() {
            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("folder")
                .to_string();
            let zip_name = format!("{dir_name}.zip");
            let (tmp, size) = zip_directory(path.clone()).await?;
            FileEntry {
                source_path: tmp.path().to_path_buf(),
                display_name: zip_name,
                size_bytes: size,
                file_type: "application/zip".to_string(),
                sha256: None,
                _temp: Some(tmp),
            }
        } else {
            let size = tokio::fs::metadata(path).await?.len();
            if size > MAX_FILE_SIZE {
                bail!(
                    "File '{}' is too large ({size} bytes; limit is {MAX_FILE_SIZE})",
                    path.display()
                );
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            // Compute SHA-256 so the receiver can verify integrity.
            let sha256 = {
                let bytes = tokio::fs::read(path).await?;
                hex::encode(Sha256::digest(&bytes))
            };
            FileEntry {
                source_path: path.clone(),
                display_name: name,
                size_bytes: size,
                file_type: mime_guess(&path.extension().and_then(|e| e.to_str()).unwrap_or("")),
                sha256: Some(sha256),
                _temp: None,
            }
        };
        entries.push((file_id, entry));
    }

    let total_bytes: u64 = entries.iter().map(|(_, e)| e.size_bytes).sum();

    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls.client_config_for_peer(&peer_fingerprint))
        .build()?;

    let base_url = format!("https://{}:{}", peer_addr.ip(), peer_addr.port());

    // ── 1. prepare-upload ────────────────────────────────────────────────────
    let files_meta: HashMap<String, FileMetadata> = entries
        .iter()
        .map(|(id, e)| {
            (
                id.clone(),
                FileMetadata {
                    id: id.clone(),
                    file_name: e.display_name.clone(),
                    size: e.size_bytes,
                    file_type: e.file_type.clone(),
                    sha256: e.sha256.clone(),
                    preview: None,
                },
            )
        })
        .collect();

    let prepare_req = PrepareUploadRequest {
        info: DeviceInfo {
            alias: sender_name.clone(),
            version: "2.0".to_string(),
            device_model: Some("PC".to_string()),
            device_type: Some("desktop".to_string()),
            fingerprint: sender_fingerprint,
            port: LOCALSEND_PORT,
            protocol: "https".to_string(),
            download: false,
            announce: None,
        },
        files: files_meta,
    };

    tracing::info!("Sending prepare-upload ({} file(s)) to {base_url}", entries.len());

    let resp = client
        .post(format!("{base_url}/api/localsend/v2/prepare-upload"))
        .json(&prepare_req)
        .send()
        .await?;

    match resp.status().as_u16() {
        403 => bail!("Peer denied the transfer"),
        s if s != 200 => bail!("Unexpected response from peer: HTTP {s}"),
        _ => {}
    }

    let PrepareUploadResponse { session_id, files: tokens } = resp.json().await?;
    tracing::info!("Transfer accepted; uploading {total_bytes} bytes total");

    // ── 2. Upload each file ──────────────────────────────────────────────────
    let start_instant = Instant::now();
    let bytes_sent_total = Arc::new(AtomicU64::new(0));

    for (file_id, entry) in &entries {
        let token = tokens
            .get(file_id)
            .ok_or_else(|| {
                anyhow::anyhow!("Peer returned no token for file '{}'", entry.display_name)
            })?
            .clone();

        let file = tokio::fs::File::open(&entry.source_path).await?;
        let base_stream = ReaderStream::with_capacity(file, CHUNK_SIZE);

        // Wrap stream to emit TransferProgress per chunk with speed/ETA.
        let event_tx_prog = event_tx.clone();
        let tid = transfer_id.to_string();
        let bst = bytes_sent_total.clone();
        let progress_stream = base_stream.map(move |chunk| {
            if let Ok(ref c) = chunk {
                let new_total = bst.fetch_add(c.len() as u64, Ordering::Relaxed) + c.len() as u64;
                let (bps, eta) = speed_eta(new_total, total_bytes, start_instant);
                let _ = event_tx_prog.try_send(AppEvent::TransferProgress {
                    transfer_id: tid.clone(),
                    bytes_done: new_total,
                    total_bytes,
                    bytes_per_sec: bps,
                    eta_secs: eta,
                });
            }
            chunk
        });

        let file_size = entry.size_bytes;
        let upload_resp = client
            .post(format!("{base_url}/api/localsend/v2/upload"))
            .query(&[
                ("sessionId", session_id.as_str()),
                ("fileId", file_id.as_str()),
                ("token", token.as_str()),
            ])
            .header("Content-Type", "application/octet-stream")
            .header("Content-Length", file_size.to_string())
            .body(reqwest::Body::wrap_stream(progress_stream))
            .send()
            .await?;

        if !upload_resp.status().is_success() {
            bail!(
                "Upload failed for '{}': HTTP {}",
                entry.display_name,
                upload_resp.status()
            );
        }
        tracing::info!("Uploaded '{}' ✓", entry.display_name);
    }

    let saved_to = entries
        .first()
        .map(|(_, e)| e.source_path.clone())
        .unwrap_or_default();

    let _ = event_tx
        .send(AppEvent::TransferComplete {
            transfer_id: transfer_id.to_string(),
            saved_to,
        })
        .await;

    tracing::info!("All {} file(s) transferred ✓", entries.len());
    Ok(())
}
