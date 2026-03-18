/// Network entry point: HTTPS server, peer discovery, and UI command loop.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::discovery::{Discovery, DiscoveryEvent};
use crate::localsend::LOCALSEND_PORT;
use crate::tls::TlsStack;
use crate::transfer::send_files;

use super::handlers::{handler_cancel, handler_device_info, handler_prepare_upload, handler_upload};
use super::state::AppState;
use super::types::{AppCommand, AppEvent, PendingMap};

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

    let mut browse_rx = discovery
        .browse(config.device_name.clone(), LOCALSEND_PORT)
        .await?;

    let event_tx_disc = event_tx.clone();
    let browse_handle = tokio::spawn(async move {
        while let Some(ev) = browse_rx.recv().await {
            match ev {
                DiscoveryEvent::PeerFound { fingerprint, alias, addr, port } => {
                    let _ = event_tx_disc
                        .send(AppEvent::PeerFound {
                            id: fingerprint,
                            name: alias,
                            addr: SocketAddr::new(addr, port),
                        })
                        .await;
                }
                DiscoveryEvent::PeerLost { fingerprint } => {
                    let _ = event_tx_disc
                        .send(AppEvent::PeerLost { id: fingerprint })
                        .await;
                }
            }
        }
    });
    let mut browse_abort = browse_handle.abort_handle();

    // ── Command loop ─────────────────────────────────────────────────────────
    while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
            AppCommand::SendFiles { peer_addr, peer_fingerprint, paths } => {
                let tls = tls.clone();
                let etx = event_tx.clone();
                let name = device_name.read().await.clone();
                let fp = fingerprint.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        send_files(peer_addr, paths, name, fp, peer_fingerprint, tls, etx).await
                    {
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
                        let mut new_rx = rx;
                        let etx = event_tx.clone();
                        let new_handle = tokio::spawn(async move {
                            while let Some(ev) = new_rx.recv().await {
                                match ev {
                                    DiscoveryEvent::PeerFound {
                                        fingerprint,
                                        alias,
                                        addr,
                                        port,
                                    } => {
                                        let _ = etx
                                            .send(AppEvent::PeerFound {
                                                id: fingerprint,
                                                name: alias,
                                                addr: SocketAddr::new(addr, port),
                                            })
                                            .await;
                                    }
                                    DiscoveryEvent::PeerLost { fingerprint } => {
                                        let _ = etx
                                            .send(AppEvent::PeerLost { id: fingerprint })
                                            .await;
                                    }
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
