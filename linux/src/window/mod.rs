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
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = event_rx.recv().await {
                if let Some(win) = win_weak.upgrade() {
                    win.dispatch_event(event);
                }
            }
        });
    }

    /// Route one `AppEvent` to the appropriate UI update.
    fn dispatch_event(&self, event: AppEvent) {
        let imp = self.imp();
        crate::ui::handle_event(
            event,
            &imp.list_box,
            &imp.stack,
            &imp.status_dot,
            &imp.progress_bar,
            &imp.peers,
            imp.cmd_tx.get().expect("cmd_tx not initialised"),
            self.upcast_ref::<libadwaita::ApplicationWindow>(),
        );
    }
}
