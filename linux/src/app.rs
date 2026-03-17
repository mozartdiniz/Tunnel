/// Network orchestration layer.
///
/// Hosts the LocalSend HTTPS server (receive side) and drives peer discovery
/// (UDP multicast). Bridges the tokio network layer with the GTK UI via
/// `AppEvent` / `AppCommand` channels.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::discovery::{Discovery, DiscoveryEvent};
use crate::inhibit::InhibitGuard;
use crate::localsend::{
    DeviceInfo, FileMetadata, PrepareUploadRequest, PrepareUploadResponse, LOCALSEND_PORT,
};
use crate::tls::TlsStack;
use crate::transfer::{sanitize_filename, send_files, speed_eta, MAX_FILE_SIZE};

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

// ── axum handlers ─────────────────────────────────────────────────────────────

/// GET /api/localsend/v2/info
pub async fn handler_device_info(
    State(state): State<Arc<AppState>>,
) -> Json<DeviceInfo> {
    let alias = state.device_name.read().await.clone();
    Json(DeviceInfo {
        alias,
        version: "2.0".to_string(),
        device_model: Some("PC".to_string()),
        device_type: Some("desktop".to_string()),
        fingerprint: state.fingerprint.clone(),
        port: LOCALSEND_PORT,
        protocol: "https".to_string(),
        download: false,
        announce: None,
    })
}

/// POST /api/localsend/v2/prepare-upload
///
/// The sender offers a list of files. We notify the UI and wait up to 60 s for
/// the user to accept or deny. Returns 200 + session/tokens on accept, 403 on deny.
pub async fn handler_prepare_upload(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PrepareUploadRequest>,
) -> axum::response::Response {
    let session_id = uuid::Uuid::new_v4().to_string();
    let sender_alias = req.info.alias.clone();
    let sender_fp = req.info.fingerprint.clone();
    let file_count = req.files.len();
    let total_bytes: u64 = req.files.values().map(|f| f.size).sum();

    // Generate a random upload token per file.
    let tokens: HashMap<String, String> = req
        .files
        .keys()
        .map(|id| (id.clone(), uuid::Uuid::new_v4().to_string()))
        .collect();

    // Register the decision channel before notifying the UI.
    let (decision_tx, decision_rx) = tokio::sync::oneshot::channel::<bool>();
    state.pending.lock().await.insert(session_id.clone(), decision_tx);

    // Use the first file for the UI notification.
    let first_name = req
        .files
        .values()
        .next()
        .map(|f| f.file_name.clone())
        .unwrap_or_else(|| "file".to_string());

    let fp_short = sender_fp[..sender_fp.len().min(16)].to_string();
    let _ = state
        .event_tx
        .send(AppEvent::IncomingRequest {
            transfer_id: session_id.clone(),
            sender_name: sender_alias,
            file_name: first_name,
            file_count,
            size_bytes: total_bytes,
            peer_fingerprint: fp_short,
        })
        .await;

    // Await user decision with a 60-second timeout.
    let accepted = match tokio::time::timeout(Duration::from_secs(60), decision_rx).await {
        Ok(Ok(v)) => v,
        _ => {
            // Timeout or channel closed — clean up the stale pending entry.
            state.pending.lock().await.remove(&session_id);
            false
        }
    };

    if !accepted {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Accepted — acquire sleep inhibitor then create the session.
    let inhibit = Arc::new(InhibitGuard::acquire("Receiving files").await);
    let session = SessionState {
        files: req.files.clone(),
        tokens: tokens.clone(),
        download_dir: state.download_dir.read().await.clone(),
        files_remaining: Arc::new(AtomicUsize::new(file_count)),
        total_bytes,
        bytes_received: Arc::new(AtomicU64::new(0)),
        start_instant: Instant::now(),
        inhibit,
    };
    state.sessions.lock().await.insert(session_id.clone(), session);

    Json(PrepareUploadResponse { session_id, files: tokens }).into_response()
}

#[derive(serde::Deserialize)]
pub struct UploadParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub token: String,
}

/// POST /api/localsend/v2/upload?sessionId=&fileId=&token=
///
/// Streams one file's bytes to disk, verifies SHA-256 if provided, atomically
/// renames to the final path. Sends `TransferComplete` only after the last file
/// in the session has been received.
pub async fn handler_upload(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UploadParams>,
    request: axum::extract::Request,
) -> axum::response::Response {
    // Look up session and validate token.
    let session = {
        let sessions = state.sessions.lock().await;
        sessions.get(&params.session_id).cloned()
    };
    let Some(session) = session else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let expected_token = session.tokens.get(&params.file_id);
    if expected_token.map(String::as_str) != Some(params.token.as_str()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(file_meta) = session.files.get(&params.file_id).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };

    if file_meta.size > MAX_FILE_SIZE {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }

    // Disk space check.
    if let Ok(available) = fs2::available_space(&session.download_dir) {
        if file_meta.size > available {
            return (StatusCode::INSUFFICIENT_STORAGE, "Not enough disk space").into_response();
        }
    }

    let safe_name = sanitize_filename(&file_meta.file_name);
    let dest_path = session.download_dir.join(&safe_name);
    let temp_path = session
        .download_dir
        .join(format!(".{}.{}.tmp", params.session_id, params.file_id));

    let dest_file = match tokio::fs::File::create(&temp_path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("Failed to create temp file: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut writer = BufWriter::new(dest_file);
    let mut hasher = Sha256::new();

    // Stream request body, write chunks to disk, track overall session progress.
    let mut body_stream = request.into_body().into_data_stream();
    while let Some(chunk) = body_stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Body stream error: {e}");
                let _ = tokio::fs::remove_file(&temp_path).await;
                return StatusCode::BAD_REQUEST.into_response();
            }
        };
        hasher.update(&chunk);
        if let Err(e) = writer.write_all(&chunk).await {
            tracing::error!("Write error: {e}");
            let _ = tokio::fs::remove_file(&temp_path).await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }

        // Update shared session progress counters and emit event.
        let total_received =
            session.bytes_received.fetch_add(chunk.len() as u64, Ordering::Relaxed)
                + chunk.len() as u64;
        let (bps, eta) = speed_eta(total_received, session.total_bytes, session.start_instant);
        let _ = state.event_tx.try_send(AppEvent::TransferProgress {
            transfer_id: params.session_id.clone(),
            bytes_done: total_received,
            total_bytes: session.total_bytes,
            bytes_per_sec: bps,
            eta_secs: eta,
        });
    }

    if let Err(e) = writer.flush().await {
        tracing::error!("Flush error: {e}");
        let _ = tokio::fs::remove_file(&temp_path).await;
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Verify SHA-256 if the sender provided it.
    let computed = hex::encode(hasher.finalize());
    if let Some(expected_sha) = &file_meta.sha256 {
        if &computed != expected_sha {
            tracing::error!("Checksum mismatch: expected {expected_sha}, got {computed}");
            let _ = tokio::fs::remove_file(&temp_path).await;
            let _ = state
                .event_tx
                .send(AppEvent::TransferError {
                    transfer_id: params.session_id.clone(),
                    message: "Checksum mismatch — transfer discarded".into(),
                })
                .await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // Atomic rename to final destination.
    if let Err(e) = tokio::fs::rename(&temp_path, &dest_path).await {
        tracing::error!("Rename failed: {e}");
        let _ = tokio::fs::remove_file(&temp_path).await;
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    tracing::info!("Received '{}' → '{}'", file_meta.file_name, dest_path.display());

    // Decrement file counter. When it hits zero this was the last file in the session.
    let prev_remaining = session.files_remaining.fetch_sub(1, Ordering::SeqCst);
    if prev_remaining == 1 {
        state.sessions.lock().await.remove(&params.session_id);
        // `session` (and its Arc<InhibitGuard>) is dropped when this handler returns.
        let _ = state
            .event_tx
            .send(AppEvent::TransferComplete {
                transfer_id: params.session_id.clone(),
                saved_to: session.download_dir.clone(),
            })
            .await;
    }

    StatusCode::OK.into_response()
}

#[derive(serde::Deserialize)]
pub struct CancelParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// POST /api/localsend/v2/cancel?sessionId=
pub async fn handler_cancel(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CancelParams>,
) -> StatusCode {
    if let Some(tx) = state.pending.lock().await.remove(&params.session_id) {
        let _ = tx.send(false);
    }
    state.sessions.lock().await.remove(&params.session_id);
    let _ = state
        .event_tx
        .send(AppEvent::TransferError {
            transfer_id: params.session_id.clone(),
            message: "Transfer cancelled by sender".into(),
        })
        .await;
    StatusCode::OK
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
            AppCommand::SendFiles { peer_addr, paths } => {
                let tls = tls.clone();
                let etx = event_tx.clone();
                let name = device_name.read().await.clone();
                let fp = fingerprint.clone();
                tokio::spawn(async move {
                    if let Err(e) = send_files(peer_addr, paths, name, fp, tls, etx).await {
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
