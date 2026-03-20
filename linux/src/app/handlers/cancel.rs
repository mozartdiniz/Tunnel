/// POST /api/localsend/v2/cancel
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;

use crate::app::state::AppState;
use crate::app::types::AppEvent;

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
    let peer_fingerprint = state
        .sessions
        .lock()
        .await
        .remove(&params.session_id)
        .map(|s| s.peer_fingerprint)
        .unwrap_or_default();
    let _ = state
        .event_tx
        .send(AppEvent::TransferError {
            transfer_id: params.session_id.clone(),
            peer_fingerprint,
            message: "Transfer cancelled by sender".into(),
        })
        .await;
    StatusCode::OK
}
