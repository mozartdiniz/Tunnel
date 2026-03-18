/// Network orchestration layer.
///
/// Hosts the LocalSend HTTPS server (receive side) and drives peer discovery
/// (UDP multicast). Bridges the tokio network layer with the GTK UI via
/// `AppEvent` / `AppCommand` channels.
mod handlers;
mod network;
mod state;
mod types;

pub use network::run_network;
pub use types::{AppCommand, AppEvent};
