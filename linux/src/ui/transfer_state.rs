/// Authoritative transfer lifecycle state for the main window.
///
/// Inspired by Warp's `UIState` pattern: a single enum replaces the scattered
/// progress-bar / status-dot mutations that were previously spread across
/// individual `AppEvent` match arms. `Window::update_transfer_ui` is the sole
/// place that translates this state into widget updates.
#[derive(Debug, Default, Clone)]
pub enum TransferState {
    /// No transfer in progress. Progress bar hidden, status dot green.
    #[default]
    Idle,

    /// Active transfer. Progress bar visible with live stats.
    Transferring {
        bytes_done: u64,
        total_bytes: u64,
        /// Rolling-average speed in bytes/second.
        bytes_per_sec: u64,
        /// Estimated seconds remaining (`None` while speed is too low to estimate).
        eta_secs: Option<u64>,
    },

    /// Background sync transfer in progress — no per-file stats shown.
    Syncing,

    /// Transfer just finished successfully. Progress bar sits at 100% briefly
    /// before `Window` transitions back to `Idle` via a 1.2 s timeout.
    Complete,

    /// Transfer failed. Error toast shown, status dot red.
    Error(String),
}
