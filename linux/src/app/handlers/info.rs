/// GET /api/localsend/v2/info
use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::app::state::AppState;
use crate::localsend::{DeviceInfo, LOCALSEND_PORT};

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
