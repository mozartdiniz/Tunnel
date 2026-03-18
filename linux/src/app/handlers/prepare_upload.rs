/// POST /api/localsend/v2/prepare-upload
///
/// The sender offers a list of files. We notify the UI and wait up to 60 s for
/// the user to accept or deny. Returns 200 + session/tokens on accept, 403 on deny.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

const DECISION_TIMEOUT_SECS: u64 = 60;

use crate::app::state::{AppState, SessionState};
use crate::app::types::AppEvent;
use crate::inhibit::InhibitGuard;
use crate::localsend::{PrepareUploadRequest, PrepareUploadResponse};

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

    // Await user decision with a timeout.
    let accepted = match tokio::time::timeout(Duration::from_secs(DECISION_TIMEOUT_SECS), decision_rx).await {
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
