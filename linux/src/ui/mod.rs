/// Stateless UI helper modules.
///
/// Each module exports pure functions — no GObject state lives here.
/// The window subclass (`crate::window`) calls into these functions and owns
/// all widget references.
mod dialogs;
mod event_handler;
mod helpers;
mod notifications;
mod peer_list;
mod preferences;

// Re-export the public surface used by `window/mod.rs` and `window/imp.rs`.
pub use dialogs::show_transfer_request;
pub use event_handler::handle_event;
pub use helpers::{format_eta, human_bytes, set_status};
pub use notifications::{send_complete_notification, send_incoming_notification};
pub use peer_list::{add_peer_row, remove_peer_row, update_stack};
pub use preferences::show_preferences;
