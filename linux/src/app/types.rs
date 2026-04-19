use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

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
        /// Full fingerprint of the remote peer — used to route progress to its row.
        peer_fingerprint: String,
        bytes_done: u64,
        total_bytes: u64,
        /// Rolling average transfer speed in bytes/second.
        bytes_per_sec: u64,
        /// Estimated seconds remaining (None while speed is too low to estimate).
        eta_secs: Option<u64>,
        is_sync: bool,
    },
    TransferComplete {
        transfer_id: String,
        peer_fingerprint: String,
        /// Download directory on the receiver side; `None` on the sender side
        /// (the sender doesn't save files locally).
        saved_to: Option<PathBuf>,
        is_sync: bool,
    },
    TransferError {
        transfer_id: String,
        peer_fingerprint: String,
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
    /// User changed sync folder in settings (None = disabled).
    SetSyncFolder(Option<PathBuf>),
    /// User pressed the refresh button — restart peer discovery.
    RefreshPeers,
}

/// Pending incoming transfers waiting for user confirmation.
pub type PendingMap = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;
