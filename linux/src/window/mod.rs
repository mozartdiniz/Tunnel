/// Public API for the main application window.
///
/// Construction:  `Window::new(app)` → present with `window.present()`
/// Initialisation:`window.setup(config, event_rx, cmd_tx)` — called once from
///                 `Application::activate()` after the channel pair is ready.
mod imp;

use std::rc::Rc;
use std::cell::RefCell;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::app::{AppCommand, AppEvent};
use crate::config::Config;
use crate::ui::TransferState;

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends libadwaita::ApplicationWindow, gtk4::ApplicationWindow,
                 gtk4::Window, gtk4::Widget,
        @implements gtk4::gio::ActionGroup, gtk4::gio::ActionMap,
                    gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget,
                    gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl Window {
    /// Create a new window attached to `app`.
    pub fn new(app: &libadwaita::Application) -> Self {
        glib::Object::builder()
            .property("application", app)
            .build()
    }

    /// Wire up the channel pair and start the async event loop.
    ///
    /// Must be called exactly once, immediately after `new()`.
    pub fn setup(
        &self,
        config: Config,
        event_rx: async_channel::Receiver<AppEvent>,
        cmd_tx: async_channel::Sender<AppCommand>,
    ) {
        let imp = self.imp();

        // Store runtime state.
        imp.cmd_tx.set(cmd_tx).expect("Window::setup called more than once");
        imp.config
            .set(Rc::new(RefCell::new(config.clone())))
            .expect("Window::setup called more than once");

        // Seed window subtitle with device name.
        imp.window_title.set_subtitle(&config.device_name);

        // Spawn async event-loop on the GTK main context.
        let win_weak = self.downgrade();
        glib::spawn_future_local(async move {
            while let Ok(event) = event_rx.recv().await {
                if let Some(win) = win_weak.upgrade() {
                    win.dispatch_event(event);
                }
            }
        });
    }

    /// Route one `AppEvent` to the appropriate UI update.
    ///
    /// Peer and request events are handled inline. Transfer events update
    /// `transfer_state` and delegate all widget mutations to `update_transfer_ui`.
    fn dispatch_event(&self, event: AppEvent) {
        let imp = self.imp();
        let cmd_tx = imp.cmd_tx.get().expect("cmd_tx not initialised");

        match event {
            AppEvent::PeerFound { id, name, addr } => {
                imp.peers.borrow_mut().insert(id.clone(), (name.clone(), addr));
                crate::ui::add_peer_row(&imp.list_box, &id, &name, addr, cmd_tx);
                crate::ui::update_stack(&imp.list_box, &imp.stack);
            }

            AppEvent::PeerLost { id } => {
                imp.peers.borrow_mut().remove(&id);
                crate::ui::remove_peer_row(&imp.list_box, &id);
                crate::ui::update_stack(&imp.list_box, &imp.stack);
            }

            AppEvent::IncomingRequest {
                transfer_id,
                sender_name,
                file_name,
                file_count,
                size_bytes,
                peer_fingerprint,
            } => {
                crate::ui::show_transfer_request(
                    self.upcast_ref::<libadwaita::ApplicationWindow>(),
                    transfer_id.clone(),
                    sender_name.clone(),
                    file_name.clone(),
                    file_count,
                    size_bytes,
                    peer_fingerprint,
                    cmd_tx,
                );
                crate::ui::send_incoming_notification(
                    &transfer_id,
                    &sender_name,
                    &file_name,
                    file_count,
                    size_bytes,
                );
            }

            AppEvent::TransferProgress {
                bytes_done,
                total_bytes,
                bytes_per_sec,
                eta_secs,
                ..
            } => {
                self.set_transfer_state(TransferState::Transferring {
                    bytes_done,
                    total_bytes,
                    bytes_per_sec,
                    eta_secs,
                });
            }

            AppEvent::TransferComplete { saved_to, .. } => {
                crate::ui::send_complete_notification(saved_to.as_deref());
                self.set_transfer_state(TransferState::Complete);
            }

            AppEvent::TransferError { message, .. } => {
                self.set_transfer_state(TransferState::Error(message));
            }
        }
    }

    /// Transition to a new transfer state and immediately re-render transfer UI.
    fn set_transfer_state(&self, state: TransferState) {
        *self.imp().transfer_state.borrow_mut() = state;
        self.update_transfer_ui();
    }

    /// Re-render all transfer-related widgets from the current `transfer_state`.
    ///
    /// This is the single place that touches `progress_bar` and `status_dot` —
    /// all other code transitions state and calls here.
    fn update_transfer_ui(&self) {
        let imp = self.imp();
        let state = imp.transfer_state.borrow().clone();

        match state {
            TransferState::Idle => {
                imp.progress_bar.set_visible(false);
                imp.progress_bar.set_fraction(0.0);
                imp.progress_bar.set_text(None);
                crate::ui::set_status(&imp.status_dot, "idle");
            }

            TransferState::Transferring {
                bytes_done,
                total_bytes,
                bytes_per_sec,
                eta_secs,
            } => {
                let fraction = if total_bytes > 0 {
                    (bytes_done as f64 / total_bytes as f64).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let speed_str = if bytes_per_sec > 0 {
                    format!("  {}ps", crate::ui::human_bytes(bytes_per_sec))
                } else {
                    String::new()
                };
                let eta_str = match eta_secs {
                    Some(s) => format!("  ETA {}", crate::ui::format_eta(s)),
                    None => String::new(),
                };

                imp.progress_bar.set_fraction(fraction);
                imp.progress_bar.set_text(Some(&format!(
                    "{} / {}  ({:.1}%){}{}",
                    crate::ui::human_bytes(bytes_done),
                    crate::ui::human_bytes(total_bytes),
                    fraction * 100.0,
                    speed_str,
                    eta_str,
                )));
                imp.progress_bar.set_visible(true);
                crate::ui::set_status(&imp.status_dot, "transfer");
            }

            TransferState::Complete => {
                imp.progress_bar.set_fraction(1.0);
                imp.progress_bar.set_visible(true);
                crate::ui::set_status(&imp.status_dot, "idle");

                // Transition back to Idle after a short pause.
                let win_weak = self.downgrade();
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(1200),
                    move || {
                        if let Some(win) = win_weak.upgrade() {
                            win.set_transfer_state(TransferState::Idle);
                        }
                    },
                );
            }

            TransferState::Error(ref message) => {
                imp.progress_bar.set_visible(false);
                imp.progress_bar.set_fraction(0.0);
                crate::ui::set_status(&imp.status_dot, "error");
                crate::ui::show_error(
                    &imp.toast_overlay,
                    &format!("Transfer failed: {message}"),
                );
            }
        }
    }
}
