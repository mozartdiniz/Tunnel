use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::discovery::Discovery;
use crate::tls::TlsStack;
use crate::transfer::{receive_file, send_file};

/// Events sent from the network layer to the GTK UI.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A new peer running Tunnel was found on the LAN.
    PeerFound {
        id: String,
        name: String,
        addr: SocketAddr,
    },
    /// A peer disappeared from the network.
    PeerLost { id: String },
    /// A remote peer wants to send us a file.
    IncomingRequest {
        transfer_id: String,
        sender_name: String,
        file_name: String,
        size_bytes: u64,
    },
    /// Transfer progress update.
    TransferProgress {
        transfer_id: String,
        bytes_done: u64,
        total_bytes: u64,
    },
    /// Transfer finished successfully.
    TransferComplete {
        transfer_id: String,
        saved_to: PathBuf,
    },
    /// Something went wrong.
    TransferError {
        transfer_id: String,
        message: String,
    },
}

/// Commands sent from the GTK UI to the network layer.
#[derive(Debug)]
pub enum AppCommand {
    /// User dragged a file onto a peer.
    SendFile {
        peer_addr: SocketAddr,
        file_path: PathBuf,
    },
    /// User clicked "Accept" on an incoming request dialog.
    AcceptTransfer { transfer_id: String },
    /// User clicked "Deny".
    DenyTransfer { transfer_id: String },
}

/// Pending incoming transfers waiting for user confirmation.
pub type PendingMap = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;

/// Entry point for the entire network stack.
/// Runs inside a dedicated tokio runtime on a background OS thread.
pub async fn run_network(
    config: Config,
    event_tx: async_channel::Sender<AppEvent>,
    cmd_rx: async_channel::Receiver<AppCommand>,
) -> Result<()> {
    // 1. Load or generate our TLS identity
    let tls = Arc::new(TlsStack::load_or_create(&config).await?);
    tracing::info!("TLS identity ready (device: {})", config.device_name);

    // 2. Bind TCP listener on a dynamic OS-assigned port
    let listener = TcpListener::bind("0.0.0.0:0").await?;
    let local_port = listener.local_addr()?.port();
    tracing::info!("Listening for incoming transfers on port {local_port}");

    // 3. Start mDNS: advertise ourselves and browse for peers
    let discovery = Discovery::new()?;
    discovery.advertise(&config.device_name, local_port)?;

    let event_tx_discovery = event_tx.clone();
    let browse_rx = discovery.browse()?;
    tokio::spawn(async move {
        loop {
            match browse_rx.recv_async().await {
                Ok(event) => handle_mdns_event(event, &event_tx_discovery).await,
                Err(_) => break,
            }
        }
    });

    // 4. Accept incoming TLS connections
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let tls_server = tls.clone();
    let config_clone = config.clone();
    let event_tx_accept = event_tx.clone();
    let pending_accept = pending.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    tracing::debug!("Incoming connection from {peer_addr}");
                    let tls = tls_server.clone();
                    let cfg = config_clone.clone();
                    let etx = event_tx_accept.clone();
                    let pend = pending_accept.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            receive_file(stream, peer_addr, tls, cfg, etx, pend).await
                        {
                            tracing::error!("Receive error from {peer_addr}: {e:#}");
                        }
                    });
                }
                Err(e) => tracing::error!("Accept error: {e}"),
            }
        }
    });

    // 5. Process commands from the UI
    while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
            AppCommand::SendFile {
                peer_addr,
                file_path,
            } => {
                let tls = tls.clone();
                let etx = event_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = send_file(peer_addr, file_path, tls, etx).await {
                        tracing::error!("Send error: {e:#}");
                    }
                });
            }
            AppCommand::AcceptTransfer { transfer_id } => {
                let mut map = pending.lock().await;
                if let Some(tx) = map.remove(&transfer_id) {
                    let _ = tx.send(true);
                }
            }
            AppCommand::DenyTransfer { transfer_id } => {
                let mut map = pending.lock().await;
                if let Some(tx) = map.remove(&transfer_id) {
                    let _ = tx.send(false);
                }
            }
        }
    }

    Ok(())
}

async fn handle_mdns_event(
    event: mdns_sd::ServiceEvent,
    event_tx: &async_channel::Sender<AppEvent>,
) {
    use mdns_sd::ServiceEvent;
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let addr = match info.get_addresses().iter().next() {
                Some(ip) => SocketAddr::new(*ip, info.get_port()),
                None => return,
            };
            let name = info
                .get_properties()
                .get("display_name")
                .map(|p| p.val_str().to_string())
                .unwrap_or_else(|| info.get_hostname().to_string());

            tracing::info!("Peer found: {name} @ {addr}");
            let _ = event_tx
                .send(AppEvent::PeerFound {
                    id: info.get_fullname().to_string(),
                    name,
                    addr,
                })
                .await;
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            tracing::info!("Peer lost: {fullname}");
            let _ = event_tx
                .send(AppEvent::PeerLost { id: fullname })
                .await;
        }
        _ => {}
    }
}
