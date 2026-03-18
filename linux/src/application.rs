/// AdwApplication subclass.
///
/// `startup()` — creates the async channel pair, launches the tokio network
///               thread, and registers application-level notification actions.
/// `activate()` — creates and presents the main window (raises it on re-open).
mod imp {
    use std::cell::RefCell;

    use gtk4::gio;
    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;
    use libadwaita::subclass::prelude::*;

    use crate::app::{AppCommand, AppEvent, run_network};
    use crate::config::Config;

    #[derive(Default)]
    pub struct Application {
        /// Consumed once in `activate()` to hand off to the window.
        event_rx: RefCell<Option<async_channel::Receiver<AppEvent>>>,
        /// Cloned into notification action handlers; also handed to the window.
        cmd_tx: RefCell<Option<async_channel::Sender<AppCommand>>>,
        /// Consumed once in `activate()`.
        config: RefCell<Option<Config>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "TunnelApplication";
        type Type = super::Application;
        type ParentType = libadwaita::Application;
    }

    impl ObjectImpl for Application {}

    impl ApplicationImpl for Application {
        fn startup(&self) {
            self.parent_startup();

            let config = Config::load().unwrap_or_default();
            let (event_tx, event_rx) = async_channel::unbounded::<AppEvent>();
            let (cmd_tx, cmd_rx) = async_channel::unbounded::<AppCommand>();

            // Run the network stack on a dedicated OS thread with its own
            // tokio runtime so it never blocks the GTK main loop.
            let config_clone = config.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create tokio runtime");
                rt.block_on(async move {
                    if let Err(e) = run_network(config_clone, event_tx, cmd_rx).await {
                        tracing::error!("Network layer crashed: {e:#}");
                    }
                });
            });

            // Register notification actions before any window is shown so
            // they fire even when the app is backgrounded.
            let app = self.obj();
            register_notification_actions(&app, cmd_tx.clone());

            *self.event_rx.borrow_mut() = Some(event_rx);
            *self.cmd_tx.borrow_mut() = Some(cmd_tx);
            *self.config.borrow_mut() = Some(config);
        }

        fn activate(&self) {
            self.parent_activate();

            let app = self.obj();

            // Raise existing window instead of creating a second one.
            if let Some(window) = app.windows().first() {
                window.present();
                return;
            }

            let event_rx = self.event_rx.borrow_mut().take()
                .expect("activate called before startup");
            let cmd_tx = self.cmd_tx.borrow_mut().take()
                .expect("activate called before startup");
            let config = self.config.borrow_mut().take()
                .expect("activate called before startup");

            let window = crate::window::Window::new(app.upcast_ref());
            window.setup(config, event_rx, cmd_tx);
            window.present();
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}

    /// Register the three notification-action handlers on the Application so
    /// they fire even when no window is focused.
    fn register_notification_actions(
        app: &super::Application,
        cmd_tx: async_channel::Sender<AppCommand>,
    ) {
        // accept-transfer ──────────────────────────────────────────────────────
        let accept = gio::SimpleAction::new(
            "accept-transfer",
            Some(&String::static_variant_type()),
        );
        accept.connect_activate(glib::clone!(@strong cmd_tx => move |_, param| {
            if let Some(id) = param.and_then(|v| v.get::<String>()) {
                let _ = cmd_tx.send_blocking(AppCommand::AcceptTransfer { transfer_id: id });
            }
        }));
        app.add_action(&accept);

        // deny-transfer ────────────────────────────────────────────────────────
        let deny = gio::SimpleAction::new(
            "deny-transfer",
            Some(&String::static_variant_type()),
        );
        deny.connect_activate(glib::clone!(@strong cmd_tx => move |_, param| {
            if let Some(id) = param.and_then(|v| v.get::<String>()) {
                let _ = cmd_tx.send_blocking(AppCommand::DenyTransfer { transfer_id: id });
            }
        }));
        app.add_action(&deny);

        // reveal-file ──────────────────────────────────────────────────────────
        let reveal = gio::SimpleAction::new(
            "reveal-file",
            Some(&String::static_variant_type()),
        );
        reveal.connect_activate(move |_, param| {
            if let Some(path_str) = param.and_then(|v| v.get::<String>()) {
                let file = gio::File::for_path(&path_str);
                let target = if std::path::Path::new(&path_str).is_dir() {
                    file
                } else {
                    file.parent().unwrap_or(file)
                };
                let _ = gio::AppInfo::launch_default_for_uri(
                    &target.uri(),
                    gio::AppLaunchContext::NONE,
                );
            }
        });
        app.add_action(&reveal);
    }
}

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends libadwaita::Application, gtk4::Application, gtk4::gio::Application,
        @implements gtk4::gio::ActionGroup, gtk4::gio::ActionMap;
}

impl Application {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", "dev.tunnel.Tunnel")
            .build()
    }
}
