/// Network entry point: HTTPS server, peer discovery, and UI command loop.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock, mpsc};

use crate::config::Config;
use crate::discovery::{scan_subnets, Discovery, DiscoveryEvent};
use crate::localsend::LOCALSEND_PORT;
use crate::sync::start_sync_watcher;
use crate::tls::TlsStack;
use crate::transfer::{send_files, SendRequest};

use super::handlers::{handler_cancel, handler_device_info, handler_prepare_upload, handler_upload};
use super::state::AppState;
use super::types::{AppCommand, AppEvent, PendingMap};

/// Convert scan results into `AppEvent::PeerFound` and update the sync peers map.
async fn emit_scan_results(
    events: Vec<DiscoveryEvent>,
    event_tx: &async_channel::Sender<AppEvent>,
    sync_peers: &Arc<RwLock<HashMap<String, SocketAddr>>>,
) {
    for ev in events {
        if let DiscoveryEvent::PeerFound { fingerprint, alias, addr, port, sync_enabled } = ev {
            let sock_addr = SocketAddr::new(addr, port);
            let _ = event_tx
                .send(AppEvent::PeerFound {
                    id: fingerprint.clone(),
                    name: alias,
                    addr: sock_addr,
                })
                .await;
            if sync_enabled {
                sync_peers.write().await.insert(fingerprint, sock_addr);
            }
        }
    }
}

/// Spawn a task that forwards `DiscoveryEvent`s from the browse channel to the UI event channel.
/// Returns an abort handle so the task can be cancelled on refresh.
fn spawn_browse_loop(
    mut rx: mpsc::Receiver<DiscoveryEvent>,
    event_tx: async_channel::Sender<AppEvent>,
    sync_peers: Arc<RwLock<HashMap<String, SocketAddr>>>,
) -> tokio::task::AbortHandle {
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                DiscoveryEvent::PeerFound { fingerprint, alias, addr, port, sync_enabled } => {
                    let sock_addr = SocketAddr::new(addr, port);
                    let _ = event_tx
                        .send(AppEvent::PeerFound {
                            id: fingerprint.clone(),
                            name: alias,
                            addr: sock_addr,
                        })
                        .await;
                    if sync_enabled {
                        sync_peers.write().await.insert(fingerprint, sock_addr);
                    } else {
                        sync_peers.write().await.remove(&fingerprint);
                    }
                }
                DiscoveryEvent::PeerLost { fingerprint } => {
                    let _ = event_tx
                        .send(AppEvent::PeerLost { id: fingerprint.clone() })
                        .await;
                    sync_peers.write().await.remove(&fingerprint);
                }
            }
        }
    })
    .abort_handle()
}

/// Returns the next path from the watcher receiver, or pends forever when no
/// watcher is active. This lets `tokio::select!` cleanly ignore the watcher arm.
async fn recv_watcher(rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<PathBuf>>) -> Option<PathBuf> {
    match rx.as_mut() {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

pub async fn run_network(
    mut config: Config,
    event_tx: async_channel::Sender<AppEvent>,
    cmd_rx: async_channel::Receiver<AppCommand>,
) -> Result<()> {
    let tls = Arc::new(TlsStack::load_or_create(&config).await?);
    tracing::info!("TLS identity ready (device: {})", config.device_name);
    let fingerprint = TlsStack::fingerprint(&tls.cert);

    // Shared mutable state (updated by SetDeviceName / SetDownloadDir / SetSyncFolder).
    let device_name = Arc::new(RwLock::new(config.device_name.clone()));
    let download_dir = Arc::new(RwLock::new(config.download_dir.clone()));
    let sync_folder = Arc::new(RwLock::new(config.sync_folder.clone()));
    let recently_synced = Arc::new(Mutex::new(HashMap::new()));
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(Mutex::new(HashMap::new()));

    // Fingerprint → SocketAddr for peers that have sync enabled.
    let sync_peers: Arc<RwLock<HashMap<String, SocketAddr>>> =
        Arc::new(RwLock::new(HashMap::new()));

    let app_state = Arc::new(AppState {
        device_name: device_name.clone(),
        fingerprint: fingerprint.clone(),
        pending: pending.clone(),
        sessions: sessions.clone(),
        event_tx: event_tx.clone(),
        download_dir: download_dir.clone(),
        sync_folder: sync_folder.clone(),
        recently_synced: recently_synced.clone(),
    });

    // ── Build axum router ────────────────────────────────────────────────────
    let router = axum::Router::new()
        .route("/api/localsend/v2/info", axum::routing::get(handler_device_info))
        .route(
            "/api/localsend/v2/prepare-upload",
            axum::routing::post(handler_prepare_upload),
        )
        .route("/api/localsend/v2/upload", axum::routing::post(handler_upload))
        .route("/api/localsend/v2/cancel", axum::routing::post(handler_cancel))
        .with_state(app_state);

    // ── Start HTTPS server ───────────────────────────────────────────────────
    let server_config = tls.make_server_config()?;
    let rustls_config =
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config));

    let addr: std::net::SocketAddr = format!("0.0.0.0:{LOCALSEND_PORT}").parse()?;
    tracing::info!("LocalSend HTTPS server binding on {addr}");

    tokio::spawn(async move {
        if let Err(e) = axum_server::bind_rustls(addr, rustls_config)
            .serve(router.into_make_service())
            .await
        {
            tracing::error!("HTTPS server error: {e:#}");
        }
    });

    // ── Peer discovery ───────────────────────────────────────────────────────
    let discovery = Discovery::new(fingerprint.clone());
    if config.sync_folder.is_some() {
        discovery.set_sync_enabled(true);
    }
    discovery.advertise(&config.device_name, LOCALSEND_PORT).await?;

    let browse_rx = discovery
        .browse(config.device_name.clone(), LOCALSEND_PORT)
        .await?;

    let mut browse_abort = spawn_browse_loop(browse_rx, event_tx.clone(), sync_peers.clone());

    // ── Startup subnet scan ──────────────────────────────────────────────────
    {
        let etx = event_tx.clone();
        let fp = fingerprint.clone();
        let sp = sync_peers.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            emit_scan_results(scan_subnets(&fp).await, &etx, &sp).await;
        });
    }

    // ── File watcher (sync) ──────────────────────────────────────────────────
    let mut watcher_rx: Option<tokio::sync::mpsc::UnboundedReceiver<PathBuf>> = None;
    let mut _active_watcher: Option<crate::sync::SyncWatcher> = None;

    if let Some(ref folder) = config.sync_folder {
        match start_sync_watcher(folder.clone()) {
            Ok((rx, w)) => {
                watcher_rx = Some(rx);
                _active_watcher = Some(w);
                tracing::info!("Sync watcher started for {}", folder.display());
            }
            Err(e) => tracing::error!("Failed to start sync watcher: {e:#}"),
        }
    }

    // ── Command + watcher loop ───────────────────────────────────────────────
    loop {
        tokio::select! {
            result = cmd_rx.recv() => {
                let Ok(cmd) = result else { break };
                match cmd {
                    AppCommand::SendFiles { peer_addr, peer_fingerprint, paths } => {
                        let tls = tls.clone();
                        let etx = event_tx.clone();
                        let req = SendRequest {
                            peer_addr,
                            paths,
                            sender_name: device_name.read().await.clone(),
                            sender_fingerprint: fingerprint.clone(),
                            peer_fingerprint,
                            sync_root: None,
                        };
                        tokio::spawn(async move {
                            if let Err(e) = send_files(req, tls, etx).await {
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
                        *device_name.write().await = new_name.clone();
                        config.device_name = new_name.clone();
                        if let Err(e) = discovery.advertise(&new_name, LOCALSEND_PORT).await {
                            tracing::warn!("Failed to re-announce after name change: {e}");
                        }
                        tracing::info!("Device name updated, re-announced");
                    }

                    AppCommand::SetDownloadDir(dir) => {
                        *download_dir.write().await = dir.clone();
                        config.download_dir = dir;
                        tracing::info!("Download dir updated");
                    }

                    AppCommand::SetSyncFolder(path) => {
                        let enabled = path.is_some();
                        *sync_folder.write().await = path.clone();
                        config.sync_folder = path.clone();

                        if let Some(ref folder) = path {
                            match start_sync_watcher(folder.clone()) {
                                Ok((rx, w)) => {
                                    watcher_rx = Some(rx);
                                    _active_watcher = Some(w);
                                    tracing::info!("Sync watcher started for {}", folder.display());
                                }
                                Err(e) => tracing::error!("Failed to start sync watcher: {e:#}"),
                            }
                        } else {
                            watcher_rx = None;
                            _active_watcher = None;
                            tracing::info!("Sync watcher stopped");
                        }

                        discovery.set_sync_enabled(enabled);
                        let name = device_name.read().await.clone();
                        if let Err(e) = discovery.advertise(&name, LOCALSEND_PORT).await {
                            tracing::warn!("Failed to re-announce after sync change: {e}");
                        }
                    }

                    AppCommand::RefreshPeers => {
                        browse_abort.abort();
                        sync_peers.write().await.clear();
                        let name = device_name.read().await.clone();
                        if let Err(e) = discovery.advertise(&name, LOCALSEND_PORT).await {
                            tracing::warn!("Failed to re-announce on refresh: {e}");
                        }
                        match discovery.browse(name, LOCALSEND_PORT).await {
                            Ok(rx) => {
                                browse_abort = spawn_browse_loop(rx, event_tx.clone(), sync_peers.clone());
                                tracing::info!("Peer discovery restarted");
                            }
                            Err(e) => tracing::error!("Failed to restart browse: {e}"),
                        }
                        let etx = event_tx.clone();
                        let fp = fingerprint.clone();
                        let sp = sync_peers.clone();
                        tokio::spawn(async move {
                            emit_scan_results(scan_subnets(&fp).await, &etx, &sp).await;
                        });
                    }
                }
            }

            path = recv_watcher(&mut watcher_rx) => {
                let Some(path) = path else { continue };
                let root = sync_folder.read().await.clone();
                let Some(root) = root else { continue };

                // Skip recently received files to break the echo loop.
                {
                    let mut rs = recently_synced.lock().await;
                    let now = std::time::Instant::now();
                    rs.retain(|_, t| now.duration_since(*t) < Duration::from_secs(10));
                    if rs.contains_key(&path) {
                        continue;
                    }
                }

                let peers = sync_peers.read().await.clone();
                for (peer_fp, peer_addr) in peers {
                    let tls = tls.clone();
                    let etx = event_tx.clone();
                    let root_c = root.clone();
                    let path_c = path.clone();
                    let name = device_name.read().await.clone();
                    let fp = fingerprint.clone();
                    tokio::spawn(async move {
                        let req = SendRequest {
                            peer_addr,
                            peer_fingerprint: peer_fp,
                            paths: vec![path_c],
                            sender_name: name,
                            sender_fingerprint: fp,
                            sync_root: Some(root_c),
                        };
                        if let Err(e) = send_files(req, tls, etx).await {
                            tracing::warn!("Sync send error: {e:#}");
                        }
                    });
                }
            }
        }
    }

    Ok(())
}
