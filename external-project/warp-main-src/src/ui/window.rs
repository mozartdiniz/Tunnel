mod actions;

pub use actions::Action;

use crate::config::PersistentConfig;
use crate::gettext::gettextf;
use crate::gettext::*;
use crate::globals;
use crate::ui::action_view::ActionView;
use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::clone;
use std::cell::RefMut;
use std::str::FromStr;

use crate::ui::application::WarpApplication;
use crate::util::error::AppError;
use crate::util::{
    TransferDirection, WormholeTransferURI, error::UiError, extract_transmit_code,
    extract_transmit_uri, future::main_async_local_infallible,
};

use super::licenses::AboutDialogLicenseExt;

mod imp {
    use super::*;
    use crate::config::PersistentConfig;
    use crate::ui::welcome_dialog::WelcomeDialog;
    use crate::util::{error::UiError, future::main_async_local_infallible};
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "window.ui")]
    pub struct WarpApplicationWindow {
        #[cfg(feature = "qr_code_scanning")]
        pub page_camera: crate::ui::camera::Camera,

        #[template_child]
        pub page_root: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub send_status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub send_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub navigation_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub send_select_file_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub send_select_folder_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub receive_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub code_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub scan_qr_code_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub action_view: TemplateChild<ActionView>,
        #[template_child]
        pub file_dialog: TemplateChild<gtk::FileDialog>,
        #[template_child]
        pub inserted_code_toast: TemplateChild<adw::Toast>,
        #[template_child]
        pub ask_abort_dialog: TemplateChild<adw::AlertDialog>,
        #[template_child]
        pub no_registered_application_error_dialog: TemplateChild<adw::AlertDialog>,

        pub action_view_showing: Cell<bool>,
        pub config: RefCell<PersistentConfig>,
        pub generated_transmit_codes: RefCell<HashSet<String>>,
        pub inserted_code_toast_showing: Cell<bool>,
        pub close_in_progress: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WarpApplicationWindow {
        const NAME: &'static str = "WarpApplicationWindow";
        type Type = super::WarpApplicationWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.bind_template_instance_callbacks();
            actions::Action::install(klass);
        }

        // You must call `Widget`'s `init_template()` within `instance_init()`.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for WarpApplicationWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Devel Profile
            #[cfg(not(feature = "demo"))]
            if globals::DEBUG_BUILD {
                obj.add_css_class("devel");
            }

            #[cfg(feature = "qr_code_scanning")]
            {
                self.scan_qr_code_button.set_visible(true);
                self.navigation_view.add(&self.page_camera);
                self.page_camera.connect_code_detected(glib::clone!(
                    #[weak(rename_to = obj)]
                    self.obj(),
                    move |_camera, data| {
                        let Ok(utf8) = std::str::from_utf8(&data) else {
                            return;
                        };

                        let Ok(uri) = utf8.parse::<WormholeTransferURI>() else {
                            return;
                        };

                        if !obj.imp().action_view_showing.get() {
                            obj.open_code_from_uri(uri);
                        }
                    }
                ));
            }

            self.config
                .replace(PersistentConfig::from_file().unwrap_or_else(
                clone!(#[strong(rename_to = obj)] self.obj(), move |err| {
                    obj.connect_visible_notify(move |window| {
                        if window.is_visible() {
                            UiError::new(&gettextf(
                                "Error loading config file “{0}”, using default config.\nError: {1}",
                                &[&PersistentConfig::path().display(),
                                &err]
                            ))
                            .handle();
                        }
                    });

                    let mut config = PersistentConfig::default();
                    // Prevent the welcome window from obscuring the error message
                    config.config.welcome_window_shown = true;
                    config
                }),
            ));

            let drop_type = gio::File::static_type();
            let drag_action = gdk::DragAction::COPY;
            let drop_target = gtk::DropTarget::new(drop_type, drag_action);
            drop_target.connect_drop(clone!(
                #[weak(rename_to = obj)]
                self.obj(),
                #[upgrade_or]
                false,
                move |_target, value, _x, _y| {
                    if let Ok(file) = value.get::<gio::File>() {
                        if let Some(path) = file.path() {
                            obj.action_view().send_file(path, obj.config().app_cfg());
                            return true;
                        }
                    }

                    false
                }
            ));
            self.send_box.add_controller(drop_target);
        }
    }

    impl WidgetImpl for WarpApplicationWindow {
        fn map(&self) {
            self.parent_map();
            let widget = self.obj();

            widget.load_window_size();

            if !self.config.borrow().welcome_window_shown {
                let welcome_window = WelcomeDialog::new();
                welcome_window.present(Some(&*widget));
            }
        }
    }

    impl WindowImpl for WarpApplicationWindow {
        // Save window state on delete event
        fn close_request(&self) -> glib::Propagation {
            let window = self.obj();
            window.save_window_size();
            window.save_config();

            if window.action_view_showing()
                && window.action_view().transfer_in_progress()
                && !self.close_in_progress.get()
            {
                main_async_local_infallible(clone!(
                    #[strong]
                    window,
                    async move {
                        if window.action_view().cancel_request().await {
                            window.action_view().cancel().await;
                            window.close();
                        } else {
                            window.imp().close_in_progress.set(false);
                        }
                    }
                ));

                // When close button is clicked a second time we will just close the window
                self.close_in_progress.set(true);
                glib::Propagation::Stop
            } else {
                // Pass close request on to the parent
                self.parent_close_request()
            }
        }
    }

    impl ApplicationWindowImpl for WarpApplicationWindow {}
    impl AdwApplicationWindowImpl for WarpApplicationWindow {}

    impl WarpApplicationWindow {
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
            let context = gtk::prelude::WidgetExt::display(&*self.obj()).app_launch_context();

            glib::MainContext::default().spawn_local(async move {
                if let Err(err) =
                    gio::AppInfo::launch_default_for_uri_future(&help_uri, Some(&context)).await
                {
                    log::error!("Error launching help: {err:?}");
                }
            });
        }
    }
}

glib::wrapper! {
    pub struct WarpApplicationWindow(ObjectSubclass<imp::WarpApplicationWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionMap, gio::ActionGroup, gtk::Root, gtk::Native, gtk::Accessible,
        gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

#[gtk::template_callbacks]
impl WarpApplicationWindow {
    pub fn new(app: &WarpApplication) -> Self {
        glib::Object::builder()
            .property("application", app)
            .property("icon-name", globals::APP_ID)
            .build()
    }

    pub fn config(&self) -> RefMut<'_, PersistentConfig> {
        self.imp().config.borrow_mut()
    }

    pub fn save_config(&self) {
        if let Err(err) = self.imp().config.borrow_mut().save() {
            UiError::new(&gettextf("Error saving configuration file: {}", &[&err])).handle();
        }
    }

    pub fn set_welcome_window_shown(&self, shown: bool) {
        self.imp().config.borrow_mut().welcome_window_shown = shown;
        self.save_config();
    }

    pub fn show_about_dialog(&self) {
        let dialog =
            adw::AboutDialog::from_appdata("app/drey/Warp/metainfo.xml", Some(globals::VERSION));

        dialog.set_developers(&[&gettext("Fina Wilke")]);
        dialog.set_artists(&[&gettext("Tobias Bernard"), &gettext("Sophie Herold")]);
        dialog.set_translator_credits(&gettext("translator-credits"));
        dialog.add_other_app(
            "org.gnome.World.PikaBackup",
            // Translators: This is a promotion and should match the app name as seen at https://apps.gnome.org/PikaBackup/
            &gettext("Pika Backup"),
            // Translators: This is a promotion and should match the short app summary as seen at https://apps.gnome.org/PikaBackup/
            &gettext("Keep your data safe"),
        );
        glib::spawn_future_local(glib::clone!(
            #[weak]
            dialog,
            async move { dialog.add_embedded_license_information().await }
        ));

        dialog.present(Some(self));
    }

    fn save_window_size(&self) {
        let imp = self.imp();

        let (width, height) = self.default_size();

        imp.config.borrow_mut().window.width = width;
        imp.config.borrow_mut().window.height = height;
    }

    fn load_window_size(&self) {
        let imp = self.imp();

        let width = imp.config.borrow().window.width;
        let height = imp.config.borrow().window.height;

        self.set_default_size(width, height);
    }

    #[template_callback]
    async fn navigation_view_visible_page_notify(&self) {
        if let Some(page) = self.imp().navigation_view.visible_page() {
            let imp = self.imp();

            if page == *self.imp().action_view {
                imp.action_view_showing.set(true);
            } else {
                imp.action_view_showing.set(false);
            }

            if page == *self.imp().page_root {
                imp.code_entry.set_text("");

                if let Some(app) = self.app() {
                    app.uninhibit_transfer();
                }

                #[cfg(feature = "qr_code_scanning")]
                self.imp().page_camera.stop();
                self.add_code_from_clipboard();
                if imp.action_view.transfer_in_progress() {
                    imp.action_view.cancel().await;
                }
            }
        }
    }

    #[template_callback]
    fn inserted_code_toast_dismissed(&self) {
        self.imp().inserted_code_toast_showing.set(false);
    }

    #[template_callback]
    async fn select_file(&self) {
        self.imp()
            .file_dialog
            .set_title(&gettext("Select File to Send"));
        self.select_file_result(self.imp().file_dialog.open_future(Some(self)).await);
    }

    #[template_callback]
    async fn select_folder(&self) {
        self.imp()
            .file_dialog
            .set_title(&gettext("Select Folder to Send"));
        self.select_file_result(
            self.imp()
                .file_dialog
                .select_folder_future(Some(self))
                .await,
        );
    }

    pub fn send_file(&self, file: &gio::File) {
        if let Some(path) = file.path() {
            log::debug!("Selected file: {}", path.display());
            self.imp()
                .action_view
                .send_file(path, self.config().app_cfg());
        } else {
            log::error!("Can't send file: path is None");
        }
    }

    fn select_file_result(&self, result: Result<gio::File, glib::Error>) {
        match result {
            Ok(file) => {
                self.send_file(&file);
            }
            Err(err) => {
                log::debug!("File chooser error: {:?}", err);
            }
        }
    }

    #[template_callback]
    fn scan_qr_code_button_clicked(&self) {
        #[cfg(feature = "qr_code_scanning")]
        self.imp().navigation_view.push(&self.imp().page_camera);
    }

    #[template_callback]
    fn code_entry_changed(&self) {
        self.imp()
            .receive_button
            .set_sensitive(!self.imp().code_entry.text().is_empty());
    }

    #[template_callback]
    pub fn receive_button_clicked(&self) {
        if self.action_view_showing() {
            return;
        }

        let text = self.imp().code_entry.text();
        let uri = extract_transmit_uri(&text).and_then(|s| WormholeTransferURI::from_str(&s).ok());
        let code = if globals::TRANSMIT_CODE_MATCH_REGEX.is_match(&text) {
            match text.parse() {
                Ok(code) => code,
                Err(err) => {
                    AppError::from(err).handle();
                    return;
                }
            }
        } else if let Some(uri) = uri {
            self.open_code_from_uri(uri);
            return;
        } else if let Some(code) = extract_transmit_code(&text) {
            code
        } else {
            UiError::new(&gettextf(
                "“{}” appears to be an invalid Transmit Code. Please try again.",
                &[&text],
            ))
            .handle();
            return;
        };

        let app_cfg = if let Some(uri) = uri {
            uri.to_app_cfg()
        } else {
            self.config().app_cfg()
        };

        self.action_view().receive_file(code, app_cfg);
    }

    pub fn action_view_showing(&self) -> bool {
        self.imp().action_view_showing.get()
    }

    pub fn transfer_in_progress(&self) -> bool {
        self.action_view_showing() && self.action_view().transfer_in_progress()
    }

    pub fn show_action_view(&self) {
        let imp = self.imp();

        let visible_page = imp.navigation_view.visible_page();

        if visible_page.as_ref() == Some(imp.page_root.upcast_ref()) {
            imp.navigation_view.push(&*imp.action_view);
        }

        #[cfg(feature = "qr_code_scanning")]
        if visible_page.as_ref() == Some(imp.page_camera.upcast_ref()) {
            imp.navigation_view
                .replace(&[imp.page_root.clone(), imp.action_view.clone().upcast()]);
        }

        if imp.inserted_code_toast_showing.get() {
            imp.inserted_code_toast.get().dismiss();
        }
    }

    pub fn navigate_home(&self) {
        log::debug!("Navigating back to main screen");
        self.imp()
            .navigation_view
            .pop_to_page(&*self.imp().page_root);
    }

    pub fn add_code(&self, code: &wormhole::Code) {
        self.imp()
            .generated_transmit_codes
            .borrow_mut()
            .insert(code.to_string());
    }

    #[template_callback]
    pub fn add_code_from_clipboard(&self) {
        let Some(stack_name) = self.imp().stack.visible_child_name() else {
            return;
        };

        if self.is_active() && !self.action_view_showing() && stack_name == "receive" {
            let obj = self.clone();
            main_async_local_infallible(async move {
                let imp = obj.imp();
                let clipboard = obj.clipboard();
                let text = clipboard.read_text_future().await;
                if let Ok(Some(text)) = text {
                    let extracted_data = if let Some(uri_str) = extract_transmit_uri(&text) {
                        if let Ok(uri) = WormholeTransferURI::from_str(&uri_str) {
                            Some((uri_str, uri.code))
                        } else {
                            None
                        }
                    } else {
                        extract_transmit_code(&text).map(|code| (code.as_ref().to_owned(), code))
                    };

                    if let Some((extracted_text, code)) = extracted_data {
                        if imp.code_entry.text() != extracted_text
                            && !imp
                                .generated_transmit_codes
                                .borrow()
                                .contains(code.as_ref())
                        {
                            let imp = obj.imp();
                            imp.code_entry.set_text(&extracted_text);
                            imp.toast_overlay.add_toast(imp.inserted_code_toast.clone());
                            imp.inserted_code_toast_showing.set(true);
                        }
                    }
                }
            });
        }
    }

    pub fn toast_overlay(&self) -> adw::ToastOverlay {
        self.imp().toast_overlay.clone()
    }

    pub fn action_view(&self) -> ActionView {
        self.imp().action_view.clone()
    }

    pub fn ask_abort_dialog(&self) -> adw::AlertDialog {
        self.imp().ask_abort_dialog.clone()
    }

    pub fn no_registered_application_error_dialog(&self, msg: &str) -> adw::AlertDialog {
        let dialog = self.imp().no_registered_application_error_dialog.clone();
        dialog.set_body(msg);
        dialog
    }

    pub fn open_code_from_uri(&self, uri: WormholeTransferURI) {
        let app_cfg = uri.to_app_cfg();
        if uri.direction == TransferDirection::Receive {
            self.imp().stack.set_visible_child_name("receive");
            self.action_view().receive_file(uri.code, app_cfg);
        } else {
            let err = UiError::new(&gettext(
                "Sending files with a preconfigured code is not yet supported",
            ));
            AppError::from(err).show_error_dialog(self);
        }
    }

    pub fn app(&self) -> Option<WarpApplication> {
        self.application().and_downcast()
    }
}
