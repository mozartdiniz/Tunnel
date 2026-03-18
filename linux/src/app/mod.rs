/// Network orchestration layer.
///
/// Hosts the LocalSend HTTPS server (receive side) and drives peer discovery
/// (UDP multicast). Bridges the tokio network layer with the GTK UI via
/// `AppEvent` / `AppCommand` channels.
mod handlers;
pub use handlers::{
    handler_cancel, handler_device_info, handler_prepare_upload, handler_upload, CancelParams,
    UploadParams,
};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize},
    Arc,
};
use std::time::Instant;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::discovery::{Discovery, DiscoveryEvent};
use crate::inhibit::InhibitGuard;
use crate::localsend::{FileMetadata, LOCALSEND_PORT};
use crate::tls::TlsStack;
use crate::transfer::send_files;

// ── Public event / command types (unchanged API — UI depends on these) ─────────

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
        /// First file name; use `file_count` to display "N files" for multi-file.
        file_name: String,
        /// Total number of files in this transfer.
        file_count: usize,
        /// Sum of all file sizes.
        size_bytes: u64,
        /// First 16 hex chars of the sender's self-reported fingerprint.
        peer_fingerprint: String,
    },
    TransferProgress {
        transfer_id: String,
        bytes_done: u64,
        total_bytes: u64,
        /// Rolling average transfer speed in bytes/second.
        bytes_per_sec: u64,
        /// Estimated seconds remaining (None while speed is too low to estimate).
        eta_secs: Option<u64>,
    },
    TransferComplete {
        transfer_id: String,
        /// Saved file path (single file) or download directory (multi-file).
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
    /// Send one or more files/folders to a peer. Directories are ZIP'd automatically.
    SendFiles {
        peer_addr: SocketAddr,
        /// Peer's announced fingerprint from UDP discovery — used as TOFU key.
        peer_fingerprint: String,
        paths: Vec<PathBuf>,
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

// ── Shared state for axum handlers ───────────────────────────────────────────

/// In-progress receive session (between prepare-upload and the last upload call).
#[derive(Clone)]
pub struct SessionState {
    pub files: HashMap<String, FileMetadata>,
    /// fileId → upload token
    pub tokens: HashMap<String, String>,
    pub download_dir: PathBuf,
    // Multi-file tracking (shared via Arc so clones stay in sync).
    pub files_remaining: Arc<AtomicUsize>,
    /// Sum of all file sizes — used for overall progress reporting.
    pub total_bytes: u64,
    /// Bytes received across all files in this session.
    pub bytes_received: Arc<AtomicU64>,
    /// When the session was created (after user accepted) — used for speed/ETA.
    pub start_instant: Instant,
    /// Sleep inhibitor held for the duration of the transfer.
    /// Released automatically when the last Arc clone is dropped (RAII).
    #[allow(dead_code)]
    pub inhibit: Arc<InhibitGuard>,
}

/// State shared between all axum request handlers (and the command loop).
pub struct AppState {
    pub device_name: Arc<RwLock<String>>,
    pub fingerprint: String,
    pub pending: PendingMap,
    pub sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    pub event_tx: async_channel::Sender<AppEvent>,
    pub download_dir: Arc<RwLock<PathBuf>>,
}

// ── Network entry point ───────────────────────────────────────────────────────

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
    let sessions: Arc<Mutex<HashMap<String, SessionState>>> = Arc::new(Mutex::new(HashMap::new()));

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
