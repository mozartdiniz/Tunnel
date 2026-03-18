/// POST /api/localsend/v2/upload
///
/// Streams one file's bytes to disk, verifies SHA-256 if provided, atomically
/// renames to the final path. Sends `TransferComplete` only after the last file
/// in the session has been received.
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::app::state::{AppState, SessionState};
use crate::app::types::AppEvent;
use crate::localsend::FileMetadata;
use crate::transfer::{sanitize_filename, speed_eta, MAX_FILE_SIZE};

#[derive(serde::Deserialize)]
pub struct UploadParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub token: String,
}

pub async fn handler_upload(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UploadParams>,
    request: axum::extract::Request,
) -> axum::response::Response {
    // ── Validation phase — no temp file created yet ───────────────────────────
    let session = {
        let sessions = state.sessions.lock().await;
        sessions.get(&params.session_id).cloned()
    };
    let Some(session) = session else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if session.tokens.get(&params.file_id).map(String::as_str) != Some(params.token.as_str()) {
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

    // ── Write phase — temp file is created; single cleanup site on error ──────
    let safe_name = sanitize_filename(&file_meta.file_name);
    let dest_path = session.download_dir.join(&safe_name);
    let temp_path = session
        .download_dir
        .join(format!(".{}.{}.tmp", params.session_id, params.file_id));

    match receive_to_disk(&state, &params, request, &session, &file_meta, &temp_path, &dest_path)
        .await
    {
        Ok(response) => response,
        Err(response) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            response
        }
    }
}

/// Streams the request body to `temp_path`, verifies checksum, then atomically
/// renames to `dest_path`. Returns `Err(response)` on any failure so the caller
/// can clean up the temp file in one place.
async fn receive_to_disk(
    state: &Arc<AppState>,
    params: &UploadParams,
    request: axum::extract::Request,
    session: &SessionState,
    file_meta: &FileMetadata,
    temp_path: &Path,
    dest_path: &Path,
) -> Result<axum::response::Response, axum::response::Response> {
    let dest_file = tokio::fs::File::create(temp_path).await.map_err(|e| {
        tracing::error!("Failed to create temp file: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;
    let mut writer = BufWriter::new(dest_file);
    let mut hasher = Sha256::new();

    // Stream request body, write chunks to disk, track overall session progress.
    let mut body_stream = request.into_body().into_data_stream();
    while let Some(chunk) = body_stream.next().await {
        let chunk = chunk.map_err(|e| {
            tracing::error!("Body stream error: {e}");
            StatusCode::BAD_REQUEST.into_response()
        })?;
        hasher.update(&chunk);
        writer.write_all(&chunk).await.map_err(|e| {
            tracing::error!("Write error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;

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

    writer.flush().await.map_err(|e| {
        tracing::error!("Flush error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    // Verify SHA-256 if the sender provided it.
    let computed = hex::encode(hasher.finalize());
    if let Some(expected_sha) = &file_meta.sha256 {
        if &computed != expected_sha {
            tracing::error!("Checksum mismatch: expected {expected_sha}, got {computed}");
            let _ = state
                .event_tx
                .send(AppEvent::TransferError {
                    transfer_id: params.session_id.clone(),
                    message: "Checksum mismatch — transfer discarded".into(),
                })
                .await;
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    // Atomic rename to final destination.
    tokio::fs::rename(temp_path, dest_path).await.map_err(|e| {
        tracing::error!("Rename failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

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
                saved_to: Some(session.download_dir.clone()),
            })
            .await;
    }

    Ok(StatusCode::OK.into_response())
}
