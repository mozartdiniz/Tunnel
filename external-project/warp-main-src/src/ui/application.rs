use crate::gettext::*;
use crate::globals;
use crate::ui::window::WarpApplicationWindow;
use crate::util::TransferDirection;
use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::clone;

mod imp {
    use super::*;
    use crate::util::WormholeTransferURI;
    use crate::util::error::{AppError, UiError};
    use glib::WeakRef;
    use std::cell::Cell;
    use std::cell::OnceCell;

    #[derive(Debug, Default)]
    pub struct WarpApplication {
        pub window: OnceCell<WeakRef<WarpApplicationWindow>>,
        pub inhibit_cookie: Cell<Option<u32>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WarpApplication {
        const NAME: &'static str = "WarpApplication";
        type Type = super::WarpApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for WarpApplication {}

    impl ApplicationImpl for WarpApplication {
        fn activate(&self) {
            log::debug!("GtkApplication<WarpApplication>::activate");
            self.parent_activate();

            if let Some(window) = self.window.get() {
                let window = window.upgrade().unwrap();
                window.present();
                return;
            }

            log::debug!("Create window");

            let window = WarpApplicationWindow::new(&self.obj());
            self.window
                .set(window.downgrade())
                .expect("Window already set.");

            self.obj().main_window().present();
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            self.activate();
            let app = self.obj();

            let Some(first) = files.first() else {
                return;
            };

            if app.main_window().transfer_in_progress() {
                let err: AppError = UiError::new(&gettext(
                    "Unable to use transfer link: another transfer already in progress",
                ))
                .into();
                err.show_error_dialog(&app.main_window());
                return;
            }

            if first.uri_scheme().as_deref() == Some("wormhole-transfer") {
                match files[0].uri().parse::<WormholeTransferURI>() {
                    Ok(uri) => {
                        app.main_window().open_code_from_uri(uri);
                    }
                    Err(err) => {
                        let err: AppError = UiError::new(&err.to_string()).into();
                        err.show_error_dialog(&app.main_window());
                    }
                }
            } else {
                // Regular file / file list
                let window = app.main_window();
                window.send_file(first);
            }
        }

        fn startup(&self) {
            log::debug!("GtkApplication<WarpApplication>::startup");
            let app = self.obj();
            self.parent_startup();

            // Set icons for shell
            gtk::Window::set_default_icon_name(globals::APP_ID);

            app.cleanup_cache();
            app.setup_gactions();
            app.setup_accels();
        }
    }

    impl GtkApplicationImpl for WarpApplication {}
    impl AdwApplicationImpl for WarpApplication {}
}

glib::wrapper! {
    pub struct WarpApplication(ObjectSubclass<imp::WarpApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionMap, gio::ActionGroup;
}

impl WarpApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", Some(globals::APP_ID))
            .property("flags", gio::ApplicationFlags::HANDLES_OPEN)
            .property("resource-base-path", Some("/app/drey/Warp/"))
            .build()
    }

    pub fn main_window(&self) -> WarpApplicationWindow {
        self.imp().window.get().unwrap().upgrade().unwrap()
    }

    pub fn cleanup_cache(&self) {
        log::debug!("Cleaning up cache dir: {}", globals::CACHE_DIR.display());
        let res = std::fs::remove_dir_all(&*globals::CACHE_DIR);
        if let Err(err) = res {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::error!(
                    "Error cleaning cache dir '{}': {}",
                    globals::CACHE_DIR.display(),
                    err
                );
            }
        }
    }

    pub fn open_help(&self, page: Option<&str>) {
        /* `help:` URIs are a Linux specific thing and won't work on Windows. There, we'll just open the path to the
         * respective HTML files and hope that it launches a browser …
         */
        let help_uri = if cfg!(target_os = "linux") {
            format!("help:warp/{}", page.unwrap_or_default())
        } else {
            let file = page.unwrap_or("index");
            let mut uri = globals::WINDOWS_BASE_PATH.clone();
            /* Hardcode the "C" language for now, so no translated help files *sigh*
             *
             * The problem is that gettext is a mess and does not provide us with a good way
             * to query the currenty used language. In theory it can do that, but the values
             * it returned on Windows did not work as they should.
             */
            uri.push(format!("share\\help\\C\\warp\\{file}.html"));
            /* People with non-UTF-8 paths will at least get a good error message */
            let mut uri = uri.to_string_lossy().into_owned();
            uri.insert_str(0, "file:///");
            uri
        };

        log::debug!("Opening '{}' to show help", help_uri);
        let context = self
            .active_window()
            .map(|w| gtk::prelude::WidgetExt::display(&w).app_launch_context());

        glib::MainContext::default().spawn_local(async move {
            if let Err(err) =
                gio::AppInfo::launch_default_for_uri_future(&help_uri, context.as_ref()).await
            {
                log::error!("Error launching help: {err:?}");
            }
        });
    }

    fn setup_gactions(&self) {
        // Help
        let action_help = gio::SimpleAction::new("help", None);
        action_help.connect_activate(clone!(
            #[weak(rename_to = app)]
            self,
            move |_, _| {
                app.open_help(None);
            }
        ));
        self.add_action(&action_help);

        // Quit
        let action_quit = gio::SimpleAction::new("quit", None);
        action_quit.connect_activate(clone!(
            #[weak(rename_to = app)]
            self,
            move |_, _| {
                // This is needed to trigger the delete event and saving the window state
                app.main_window().close();
                app.quit();
            }
        ));
        self.add_action(&action_quit);
    }

    // Sets up keyboard shortcuts
    fn setup_accels(&self) {
        self.set_accels_for_action("app.help", &["F1"]);
        self.set_accels_for_action("app.quit", &["<Control>q"]);
    }

    pub fn run(&self) {
        log::info!("Warp ({})", globals::APP_ID);
        log::info!("Version: {}", globals::VERSION);
        log::info!("Datadir: {}", globals::PKGDATADIR);

        ApplicationExtManual::run(self);
    }

    pub fn inhibit_transfer(&self, transfer_direction: TransferDirection) {
        let inhibit_reason = match transfer_direction {
            TransferDirection::Send => gettext("Sending a File"),
            TransferDirection::Receive => gettext("Receiving a File"),
        };

        let inhibit = self.inhibit(
            Some(&self.main_window()),
            gtk::ApplicationInhibitFlags::LOGOUT | gtk::ApplicationInhibitFlags::SUSPEND,
            Some(&inhibit_reason),
        );

        self.imp().inhibit_cookie.set(Some(inhibit));
    }

    pub fn uninhibit_transfer(&self) {
        if let Some(cookie) = self.imp().inhibit_cookie.take() {
            self.uninhibit(cookie);
        }
    }

    pub fn is_flatpak() -> bool {
        if let Ok(var) = std::env::var("FLATPAK_ID") {
            var == globals::APP_ID
        } else {
            false
        }
    }
}

impl Default for WarpApplication {
    fn default() -> Self {
        assert!(
            gtk::is_initialized_main_thread(),
            "Calling gio::Application::default from non-main thread"
        );

        gio::Application::default()
            .expect("Application not initialized")
            .downcast::<WarpApplication>()
            .expect("Application is wrong subclass")
    }
}
