/// HTTP endpoint handlers for the LocalSend v2 receive side.
///
/// These handlers are mounted by `run_network` in the axum router and share
/// state via `Arc<AppState>`.
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

use crate::inhibit::InhibitGuard;
use crate::localsend::{DeviceInfo, PrepareUploadRequest, PrepareUploadResponse, LOCALSEND_PORT};
use crate::transfer::{sanitize_filename, speed_eta, MAX_FILE_SIZE};
use super::{AppEvent, AppState, SessionState};

// ── GET /api/localsend/v2/info ────────────────────────────────────────────────

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

// ── POST /api/localsend/v2/prepare-upload ─────────────────────────────────────
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

// ── POST /api/localsend/v2/upload ─────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct UploadParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub token: String,
}

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

// ── POST /api/localsend/v2/cancel ─────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct CancelParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

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
