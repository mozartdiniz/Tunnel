use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize},
    Arc,
};
use std::time::Instant;

use tokio::sync::{Mutex, RwLock};

use crate::inhibit::InhibitGuard;
use crate::localsend::FileMetadata;

use super::types::{AppEvent, PendingMap};

/// In-progress receive session (between prepare-upload and the last upload call).
#[derive(Clone)]
pub struct SessionState {
    /// Full fingerprint of the peer who initiated this session (sender side).
    pub peer_fingerprint: String,
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
