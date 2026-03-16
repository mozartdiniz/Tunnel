use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::discovery::Discovery;
use crate::tls::TlsStack;
use crate::transfer::{receive_file, send_file};

/// Events sent from the network layer to the GTK UI.
#[derive(Debug, Clone)]
pub enum AppEvent {
    PeerFound {
        id: String,
        name: String,
        addr: SocketAddr,
    },
    PeerLost { id: String },
    IncomingRequest {
        transfer_id: String,
        sender_name: String,
        file_name: String,
        size_bytes: u64,
        /// First 16 hex chars of the sender's TLS cert SHA-256 fingerprint.
        peer_fingerprint: String,
    },
    TransferProgress {
        transfer_id: String,
        bytes_done: u64,
        total_bytes: u64,
    },
    TransferComplete {
        transfer_id: String,
        saved_to: PathBuf,
    },
    TransferError {
        transfer_id: String,
        message: String,
    },
}

/// Commands sent from the GTK UI to the network layer.
#[derive(Debug)]
pub enum AppCommand {
    SendFile {
        peer_addr: SocketAddr,
        file_path: PathBuf,
    },
    AcceptTransfer { transfer_id: String },
    DenyTransfer { transfer_id: String },
    /// User changed device name in settings.
    SetDeviceName(String),
    /// User changed download folder in settings.
    SetDownloadDir(PathBuf),
    /// User pressed the refresh button — restart peer discovery.
    RefreshPeers,
}

/// Pending incoming transfers waiting for user confirmation.
pub type PendingMap = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;

pub async fn run_network(
    mut config: Config,
    event_tx: async_channel::Sender<AppEvent>,
    cmd_rx: async_channel::Receiver<AppCommand>,
) -> Result<()> {
    let tls = Arc::new(TlsStack::load_or_create(&config).await?);
    tracing::info!("TLS identity ready (device: {})", config.device_name);

    let listener = TcpListener::bind("0.0.0.0:0").await?;
    let local_port = listener.local_addr()?.port();
    tracing::info!("Listening on port {local_port}");

    let discovery = Discovery::new()?;

    // own_fullname is shared so the mDNS task can always filter the latest name
    let own_fullname = Arc::new(RwLock::new(
        discovery.advertise(&config.device_name, local_port)?,
    ));

    // Spawn mDNS browsing task
    let browse_rx = discovery.browse()?;
    let event_tx_mdns = event_tx.clone();
    let own_fn = own_fullname.clone();
    let browse_handle = tokio::spawn(async move {
        loop {
            match browse_rx.recv_async().await {
                Ok(event) => {
                    let current_own = own_fn.read().await.clone();
                    handle_mdns_event(event, &current_own, &event_tx_mdns).await;
                }
                Err(_) => break,
            }
        }
    });
    let mut browse_abort = browse_handle.abort_handle();

    // Spawn incoming connection acceptor
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let tls_server = tls.clone();
    let event_tx_accept = event_tx.clone();
    let pending_accept = pending.clone();

    // config.download_dir needs to be updated when user changes it.
    // We share it via Arc<RwLock<PathBuf>>.
    let download_dir = Arc::new(RwLock::new(config.download_dir.clone()));

    let download_dir_accept = download_dir.clone();
    let device_name_accept = Arc::new(RwLock::new(config.device_name.clone()));
    let device_name_server = device_name_accept.clone();

    let config_base = config.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    let tls = tls_server.clone();
                    let etx = event_tx_accept.clone();
                    let pend = pending_accept.clone();
                    let dir = download_dir_accept.read().await.clone();
                    let name = device_name_server.read().await.clone();
                    let mut cfg = config_base.clone();
                    cfg.download_dir = dir;
                    cfg.device_name = name;
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

    // Process commands from the UI
    while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
            AppCommand::SendFile {
                peer_addr,
                file_path,
            } => {
                let tls = tls.clone();
                let etx = event_tx.clone();
                let name = device_name_accept.read().await.clone();
                tokio::spawn(async move {
                    if let Err(e) = send_file(peer_addr, file_path, name, tls, etx).await {
                        tracing::error!("Send error: {e:#}");
                    }
                });
            }

            AppCommand::AcceptTransfer { transfer_id } => {
                if let Some(tx) = pending.lock().await.remove(&transfer_id) {
                    let _ = tx.send(true);
                }
            }

            AppCommand::DenyTransfer { transfer_id } => {
                if let Some(tx) = pending.lock().await.remove(&transfer_id) {
                    let _ = tx.send(false);
                }
            }

            AppCommand::SetDeviceName(new_name) => {
                let old = own_fullname.read().await.clone();
                if let Err(e) = discovery.unregister(&old) {
                    tracing::warn!("Failed to unregister old mDNS service: {e}");
                }
                match discovery.advertise(&new_name, local_port) {
                    Ok(new_fullname) => {
                        *own_fullname.write().await = new_fullname;
                        *device_name_accept.write().await = new_name.clone();
                        config.device_name = new_name;
                        tracing::info!("Device name updated, mDNS re-announced");
                    }
                    Err(e) => tracing::error!("Failed to re-advertise: {e}"),
                }
            }

            AppCommand::SetDownloadDir(dir) => {
                *download_dir.write().await = dir.clone();
                config.download_dir = dir;
                tracing::info!("Download dir updated");
            }

            AppCommand::RefreshPeers => {
                browse_abort.abort();
                match discovery.browse() {
                    Ok(rx) => {
                        let own_fn = own_fullname.clone();
                        let etx = event_tx.clone();
                        let new_handle = tokio::spawn(async move {
                            loop {
                                match rx.recv_async().await {
                                    Ok(ev) => {
                                        let own = own_fn.read().await.clone();
                                        handle_mdns_event(ev, &own, &etx).await;
                                    }
                                    Err(_) => break,
                                }
                            }
                        });
                        browse_abort = new_handle.abort_handle();
                        tracing::info!("Peer discovery restarted");
                    }
                    Err(e) => tracing::error!("Failed to restart browse: {e}"),
                }
            }
        }
    }

    Ok(())
}

async fn handle_mdns_event(
    event: mdns_sd::ServiceEvent,
    own_fullname: &str,
    event_tx: &async_channel::Sender<AppEvent>,
) {
    use mdns_sd::ServiceEvent;
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let fullname = info.get_fullname().to_string();

            // Filter self
            if fullname == own_fullname {
                return;
            }

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
                    id: fullname,
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
