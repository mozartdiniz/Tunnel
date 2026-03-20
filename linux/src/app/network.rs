/// Network entry point: HTTPS server, peer discovery, and UI command loop.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock, mpsc};

use crate::config::Config;
use crate::discovery::{scan_subnets, Discovery, DiscoveryEvent};
use crate::localsend::LOCALSEND_PORT;
use crate::tls::TlsStack;
use crate::transfer::{send_files, SendRequest};

use super::handlers::{handler_cancel, handler_device_info, handler_prepare_upload, handler_upload};
use super::state::AppState;
use super::types::{AppCommand, AppEvent, PendingMap};

/// Convert scan results into `AppEvent::PeerFound` and send them to the UI channel.
async fn emit_scan_results(
    events: Vec<DiscoveryEvent>,
    event_tx: &async_channel::Sender<AppEvent>,
) {
    for ev in events {
        if let DiscoveryEvent::PeerFound { fingerprint, alias, addr, port } = ev {
            let _ = event_tx
                .send(AppEvent::PeerFound {
                    id: fingerprint,
                    name: alias,
                    addr: SocketAddr::new(addr, port),
                })
                .await;
        }
    }
}

/// Spawn a task that forwards `DiscoveryEvent`s from the browse channel to the UI event channel.
/// Returns an abort handle so the task can be cancelled on refresh.
fn spawn_browse_loop(
    mut rx: mpsc::Receiver<DiscoveryEvent>,
    event_tx: async_channel::Sender<AppEvent>,
) -> tokio::task::AbortHandle {
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                DiscoveryEvent::PeerFound { fingerprint, alias, addr, port } => {
                    let _ = event_tx
                        .send(AppEvent::PeerFound {
                            id: fingerprint,
                            name: alias,
                            addr: SocketAddr::new(addr, port),
                        })
                        .await;
                }
                DiscoveryEvent::PeerLost { fingerprint } => {
                    let _ = event_tx
                        .send(AppEvent::PeerLost { id: fingerprint })
                        .await;
                }
            }
        }
    })
    .abort_handle()
}

pub async fn run_network(
    mut config: Config,
    event_tx: async_channel::Sender<AppEvent>,
    cmd_rx: async_channel::Receiver<AppCommand>,
) -> Result<()> {
    let tls = Arc::new(TlsStack::load_or_create(&config).await?);
    tracing::info!("TLS identity ready (device: {})", config.device_name);
    let fingerprint = TlsStack::fingerprint(&tls.cert);

    // Shared mutable state (updated by SetDeviceName / SetDownloadDir commands).
    let device_name = Arc::new(RwLock::new(config.device_name.clone()));
    let download_dir = Arc::new(RwLock::new(config.download_dir.clone()));
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(Mutex::new(HashMap::new()));

    let app_state = Arc::new(AppState {
        device_name: device_name.clone(),
        fingerprint: fingerprint.clone(),
        pending: pending.clone(),
        sessions: sessions.clone(),
        event_tx: event_tx.clone(),
        download_dir: download_dir.clone(),
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
    discovery.advertise(&config.device_name, LOCALSEND_PORT).await?;

    let browse_rx = discovery
        .browse(config.device_name.clone(), LOCALSEND_PORT)
        .await?;

    let mut browse_abort = spawn_browse_loop(browse_rx, event_tx.clone());

    // ── Startup subnet scan ──────────────────────────────────────────────────
    // Give multicast 5 seconds to find peers before falling back to a scan.
    // This catches peers on a different subnet (e.g. Ethernet ↔ Wi-Fi) without
    // forcing a scan on every launch when multicast works fine.
    {
        let etx = event_tx.clone();
        let fp = fingerprint.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            emit_scan_results(scan_subnets(&fp).await, &etx).await;
        });
    }

    // ── Command loop ─────────────────────────────────────────────────────────
    while let Ok(cmd) = cmd_rx.recv().await {
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

            AppCommand::RefreshPeers => {
                browse_abort.abort();
                let name = device_name.read().await.clone();
                if let Err(e) = discovery.advertise(&name, LOCALSEND_PORT).await {
                    tracing::warn!("Failed to re-announce on refresh: {e}");
                }
                match discovery.browse(name, LOCALSEND_PORT).await {
                    Ok(rx) => {
                        browse_abort = spawn_browse_loop(rx, event_tx.clone());
                        tracing::info!("Peer discovery restarted");
                    }
                    Err(e) => tracing::error!("Failed to restart browse: {e}"),
                }
                // Also scan subnets immediately — catches peers multicast can't reach.
                let etx = event_tx.clone();
                let fp = fingerprint.clone();
                tokio::spawn(async move {
                    emit_scan_results(scan_subnets(&fp).await, &etx).await;
                });
            }
        }
    }

    Ok(())
}
