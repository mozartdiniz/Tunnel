use super::fs;
use super::progress::FileTransferProgress;
use crate::gettext::*;
use crate::ui::fs::safe_persist_tempfile;
use crate::ui::window::WarpApplicationWindow;
use crate::util::error::*;
use crate::util::future::*;
use crate::util::{TransferDirection, WormholeTransferURI, show_dir};
use crate::{WarpApplication, globals};
use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::clone;
use std::ffi::OsString;
use std::fmt::Debug;
use std::future::Future;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

// 5 seconds timeout
const TIMEOUT_MS: u64 = 5000;

#[derive(Debug)]
pub enum UIState {
    Initial,
    Archive(OsString),
    RequestCode,
    HasCode(WormholeTransferURI),
    Connected,
    AskConfirmation(String, u64),
    Transmitting(String, wormhole::transit::TransitInfo),
    Done(OsString),
    Error(AppError),
}

/// We are only interested about the state, not the context information
impl PartialEq for UIState {
    fn eq(&self, other: &Self) -> bool {
        match self {
            UIState::Initial => matches!(other, UIState::Initial),
            UIState::Archive(_) => matches!(other, UIState::Archive(..)),
            UIState::RequestCode => matches!(other, UIState::RequestCode),
            UIState::HasCode(..) => matches!(other, UIState::HasCode(..)),
            UIState::Connected => matches!(other, UIState::Connected),
            UIState::AskConfirmation(..) => matches!(other, UIState::AskConfirmation(..)),
            UIState::Transmitting(..) => matches!(other, UIState::Transmitting(..)),
            UIState::Done(_) => matches!(other, UIState::Done(..)),
            UIState::Error(_) => matches!(other, UIState::Error(..)),
        }
    }
}

impl Default for UIState {
    fn default() -> Self {
        Self::Initial
    }
}

/// Mutable state for the `ActionView`
#[derive(Debug)]
pub struct UIContext {
    /// ID of the timer that runs the indeterminate progress
    pub progress_timeout_source_id: Option<glib::source::SourceId>,

    /// When sending a message to the cancel sender the whole process gets aborted
    pub cancel_sender: async_broadcast::Sender<()>,
    pub cancel_receiver: async_broadcast::Receiver<()>,

    /// We will send a message to this channel when the transfer was cancelled successfully
    pub cancellation_complete_sender: async_broadcast::Sender<()>,
    pub cancellation_complete_receiver: async_broadcast::Receiver<()>,

    /// Send a message to this sender to continue the process after the confirmation question
    pub continue_sender: async_broadcast::Sender<Option<PathBuf>>,
    pub continue_receiver: async_broadcast::Receiver<Option<PathBuf>>,

    /// User initiated cancel
    pub canceled: bool,

    /// Full path to the currently being received / sent file
    pub file_path: Option<PathBuf>,

    /// File path to the last file that was received successfully
    pub file_path_received_successfully: Option<PathBuf>,

    /// The user facing name of the file being received / send
    pub file_name: Option<OsString>,

    /// Whether we are currently sending or receiving
    pub direction: TransferDirection,

    /// The current UI mode
    pub ui_state: Rc<UIState>,

    /// Handle to the progress calculation
    pub progress: Option<FileTransferProgress>,

    /// The rendezvous url in use
    pub rendezvous_url: url::Url,

    /// The transit url in use
    pub relay_hints: Vec<wormhole::transit::RelayHint>,
}

impl Default for UIContext {
    fn default() -> Self {
        let (cancel_sender, cancel_receiver) = async_broadcast::broadcast(1);
        let (continue_sender, continue_receiver) = async_broadcast::broadcast(1);
        let (cancellation_complete_sender, cancellation_complete_receiver) =
            async_broadcast::broadcast(1);

        Self {
            progress_timeout_source_id: None,
            cancel_sender,
            cancel_receiver,
            cancellation_complete_sender,
            cancellation_complete_receiver,
            continue_sender,
            continue_receiver,
            canceled: false,
            file_path: None,
            file_path_received_successfully: None,
            file_name: None,
            direction: TransferDirection::default(),
            ui_state: Rc::default(),
            progress: None,
            relay_hints: globals::WORMHOLE_DEFAULT_TRANSIT_RELAY_HINTS.clone(),
            rendezvous_url: globals::WORMHOLE_DEFAULT_RENDEZVOUS_SERVER.clone(),
        }
    }
}

static TRANSIT_ABILITIES: wormhole::transit::Abilities = wormhole::transit::Abilities::ALL;

mod imp {
    use super::*;
    use gtk::gdk::AppLaunchContext;
    use std::cell::RefCell;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "action_view.ui")]
    pub struct ActionView {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,

        #[template_child]
        pub status_page_progress: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub status_page_code: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub status_page_ask_confirmation: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub status_page_success: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub status_page_error: TemplateChild<adw::StatusPage>,

        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub open_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub open_dir_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub copy_error_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub progress_bar: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub code_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub save_as_file_dialog: TemplateChild<gtk::FileDialog>,
        #[template_child]
        pub code_image: TemplateChild<gtk::Image>,
        #[template_child]
        pub code_description: TemplateChild<gtk::Label>,
        #[template_child]
        pub code_detail_label: TemplateChild<gtk::Label>,

        pub context: RefCell<UIContext>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ActionView {
        const NAME: &'static str = "ActionView";
        type Type = super::ActionView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        // You must call `Widget`'s `init_template()` within `instance_init()`.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ActionView {}

    impl WidgetImpl for ActionView {}
    impl NavigationPageImpl for ActionView {}

    #[gtk::template_callbacks]
    impl ActionView {
        #[template_callback]
        fn back_button_clicked(&self) {
            self.obj().window().navigate_home();
        }

        #[template_callback]
        async fn cancel_button_clicked(&self) {
            if self.obj().cancel_request().await {
                self.obj().cancel().await;
            }
        }

        #[template_callback]
        async fn accept_transfer_button_clicked(&self) {
            let continue_sender = self.context.borrow().continue_sender.clone();
            continue_sender.broadcast(None).await.unwrap();
        }

        #[template_callback]
        async fn save_as_button_clicked(&self) {
            match self
                .save_as_file_dialog
                .save_future(Some(&self.obj().window()))
                .await
            {
                Ok(file) => {
                    if let Some(path) = file.path() {
                        log::debug!("Selected path: '{}'", path.display());
                        let continue_sender = self.context.borrow().continue_sender.clone();
                        continue_sender.broadcast(Some(path)).await.unwrap();
                    }
                }
                Err(err) => {
                    log::debug!("Save As file chooser error: {:?}", err);
                }
            }
        }

        #[template_callback]
        fn code_copy_button_clicked(&self) {
            let UIState::HasCode(uri) = &*self.obj().ui_state() else {
                return;
            };

            let window = self.obj().window();
            let clipboard = window.clipboard();

            clipboard.set_text(uri.code.as_ref());

            // Translators: Notification when clicking on "Copy Code to Clipboard" button
            let toast = adw::Toast::new(&gettext("Copied Code to Clipboard"));
            toast.set_timeout(3);
            toast.set_priority(adw::ToastPriority::Normal);
            window.toast_overlay().add_toast(toast);
        }

        #[template_callback]
        fn link_copy_button_clicked(&self) {
            let UIState::HasCode(uri) = &*self.obj().ui_state() else {
                return;
            };

            let window = self.obj().window();
            let clipboard = window.clipboard();
            clipboard.set_text(&uri.create_uri());

            // Translators: Notification when clicking on "Copy Link to Clipboard" button
            let toast = adw::Toast::new(&gettext("Copied Link to Clipboard"));
            toast.set_timeout(3);
            toast.set_priority(adw::ToastPriority::Normal);
            window.toast_overlay().add_toast(toast);
        }

        #[template_callback]
        fn copy_error_button_clicked(&self) {
            let window = self.obj().window();

            let toast = match &*self.context.borrow().ui_state {
                UIState::Error(error) => {
                    let msg = format!("{error}");
                    window.clipboard().set_text(&msg);

                    adw::Toast::new(&gettext("Copied Error to Clipboard"))
                }
                _ => adw::Toast::new(&gettext("No error available")),
            };

            toast.set_timeout(3);
            toast.set_priority(adw::ToastPriority::Normal);
            window.toast_overlay().add_toast(toast);
        }

        #[template_callback]
        fn code_entry_select_all(entry: &gtk::Entry) {
            // Select all text when entry is focused
            if let Some(delegate) = entry.delegate() {
                if delegate.has_focus() {
                    entry.select_region(0, -1);
                }
            }
        }

        #[template_callback]
        async fn open_button_clicked(&self) {
            let maybe_path = self
                .context
                .borrow()
                .file_path_received_successfully
                .clone();

            if let Some(filename) = maybe_path {
                let uri = glib::filename_to_uri(filename.clone(), None);
                if let Ok(uri) = uri {
                    log::debug!("Opening file with uri '{}'", uri);
                    let none: Option<&AppLaunchContext> = None;
                    let res = gio::AppInfo::launch_default_for_uri(&uri, none);
                    if let Err(err) = res {
                        log::error!("Error opening file: {}", err);
                        let window = self.obj().window();

                        let answer = self
                            .obj()
                            .window()
                            .no_registered_application_error_dialog(err.message())
                            .choose_future(&window)
                            .await;

                        if answer == "show-in-folder" {
                            if let Err(err) = show_dir(&filename).await {
                                log::error!("Error showing directory: {}", err);
                                err.handle();
                            }
                        }
                    }
                } else {
                    log::error!("Filename to open is not a valid uri");
                }
            } else {
                log::error!("Open button clicked but no filename set");
            };
        }

        #[template_callback]
        async fn open_dir_button_clicked(&self) {
            let filename = self
                .context
                .borrow_mut()
                .file_path_received_successfully
                .clone();
            if let Some(filename) = filename {
                if let Err(err) = show_dir(&filename).await {
                    err.handle();
                }
            };
        }

        pub fn send_notification_if_background(
            &self,
            id: Option<&str>,
            notification: &gio::Notification,
        ) {
            if let Some(app) = self.obj().app() {
                if let Some(window) = app.active_window() {
                    if window.is_active() {
                        return;
                    }
                }

                app.send_notification(id, notification);
            }
        }
    }
}

glib::wrapper! {
    pub struct ActionView(ObjectSubclass<imp::ActionView>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ActionView {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn window(&self) -> WarpApplicationWindow {
        self.root()
            .expect("ActionView must be in a window")
            .downcast()
            .expect("ActionView may only be situated in WarpApplicationWindow")
    }

    fn app(&self) -> Option<WarpApplication> {
        self.window().app()
    }

    fn set_ui_state(&self, ui_state: UIState) {
        self.imp().context.borrow_mut().ui_state = Rc::new(ui_state);
        self.update_ui();
    }

    fn ui_state(&self) -> Rc<UIState> {
        self.imp().context.borrow().ui_state.clone()
    }

    fn set_transfer_direction(&self, direction: TransferDirection) {
        self.imp().context.borrow_mut().direction = direction;
    }

    fn transfer_direction(&self) -> TransferDirection {
        self.imp().context.borrow().direction
    }

    fn enable_back_button(&self, can_pop: bool) {
        self.imp().cancel_button.set_sensitive(!can_pop);
        self.imp().cancel_button.set_visible(!can_pop);

        adw::NavigationPage::set_can_pop(self.upcast_ref(), can_pop);

        if can_pop {
            self.remove_css_class("cancel-button-visible");
        } else {
            self.add_css_class("cancel-button-visible");
        }
    }

    fn update_ui(&self) {
        let imp = self.imp();
        let direction = self.transfer_direction();
        let ui_state = self.ui_state();

        match &*ui_state {
            UIState::Initial => {
                imp.stack.set_visible_child(&*imp.status_page_progress);
                imp.status_page_progress
                    .set_icon_name(Some("arrows-questionmark-symbolic"));
                imp.progress_bar.set_show_text(false);

                super::pride::apply_seasonal_style(&*imp.progress_bar);
                self.enable_back_button(false);
                self.show_progress_indeterminate(true);
            }
            UIState::Archive(filename) => match direction {
                TransferDirection::Send => {
                    imp.stack.set_visible_child(&*imp.status_page_progress);
                    imp.status_page_progress
                        .set_icon_name(Some("drawer-symbolic"));
                    // Translators: Title
                    imp.status_page_progress
                        .set_title(&gettext("Creating Archive"));
                    imp.status_page_progress
                        // Translators: Description
                        .set_description(Some(&gettextf(
                            "Compressing folder “{}”",
                            &[&filename.to_string_lossy()],
                        )));
                    imp.progress_bar.set_text(None);
                    imp.progress_bar.set_show_text(true);
                }
                TransferDirection::Receive => {
                    // We don't create archives here
                }
            },
            UIState::RequestCode => match direction {
                TransferDirection::Send => {
                    imp.stack.set_visible_child(&*imp.status_page_progress);
                    imp.status_page_progress
                        .set_icon_name(Some("arrows-questionmark-symbolic"));
                    // Translators: Title
                    imp.status_page_progress.set_title(&gettext("Connecting"));
                    imp.status_page_progress
                        // Translators: Description, Filename
                        .set_description(Some(&gettext("Requesting file transfer")));
                    imp.progress_bar.set_text(None);
                    imp.progress_bar.set_show_text(false);
                }
                TransferDirection::Receive => {}
            },
            UIState::HasCode(uri) => {
                match direction {
                    TransferDirection::Send => {
                        imp.stack.set_visible_child(&*imp.status_page_code);
                        imp.code_image.set_paintable(Some(&uri.to_paintable_qr()));

                        let filename = imp
                            .context
                            .borrow()
                            .file_name
                            .clone()
                            .unwrap_or_else(|| "?".into());

                        imp.code_description.set_label(&gettextf(
                            // Translators: Description, argument is filename
                            "Ready to send “{}”.",
                            &[&filename.to_string_lossy()],
                        ));

                        // Translators: Help dialog line 1, Code words and QR code visible,
                        let mut description = gettext(
                            "The receiver needs to enter or scan this code to begin the file transfer.",
                        );
                        description += "\n\n";
                        // Translators: Help dialog line 2, Argument is a list of apps that support the QR code standard.
                        description += &gettextf(
                            "The QR code is compatible with the following apps: {}.",
                            &[&"Warp, Wormhole (Android), iyox Wormhole (Android)"],
                        );

                        if imp.context.borrow().rendezvous_url
                            != *globals::WORMHOLE_DEFAULT_RENDEZVOUS_SERVER
                        {
                            description += "\n";
                            description += &gettext(
                                "You have entered a custom rendezvous server URL in preferences. Please verify the receiver also uses the same rendezvous server.",
                            );
                        }

                        description += "\n\n";
                        description +=
                            &gettext("Click the QR code to copy the link to the clipboard.");

                        imp.code_detail_label.set_label(&description);

                        #[cfg(not(feature = "demo"))]
                        imp.code_entry.set_text(uri.code.as_ref());
                        #[cfg(feature = "demo")]
                        imp.code_entry.set_text("0-code-words");

                        imp.code_entry
                            .set_max_width_chars(imp.code_entry.text().len() as i32);
                    }
                    TransferDirection::Receive => {
                        imp.stack.set_visible_child(&*imp.status_page_progress);
                        imp.status_page_progress
                            .set_icon_name(Some("arrows-questionmark-symbolic"));
                        // Translators: Title
                        imp.status_page_progress.set_title(&gettext("Connecting"));
                        imp.status_page_progress.set_description(Some(&gettextf(
                            // Translators: Description, Transfer Code
                            "Connecting to peer with code “{}”",
                            &[&uri.code],
                        )));
                    }
                }
            }
            UIState::Connected => {
                // Translators: Title
                imp.stack.set_visible_child(&*imp.status_page_progress);
                imp.status_page_progress
                    .set_title(&gettext("Connected to Peer"));

                self.show_progress_indeterminate(true);
                imp.progress_bar.set_show_text(false);

                match direction {
                    TransferDirection::Send => {
                        imp.status_page_progress
                            // Translators: Description
                            .set_description(Some(&gettext("Preparing to send file")));
                        imp.status_page_progress
                            .set_icon_name(Some("send-to-symbolic"));
                    }
                    TransferDirection::Receive => {
                        imp.status_page_progress
                            // Translators: Description
                            .set_description(Some(&gettext("Preparing to receive file")));
                        imp.status_page_progress
                            .set_icon_name(Some("folder-download-symbolic"));
                    }
                }
            }
            UIState::AskConfirmation(filename, size) => {
                imp.stack
                    .set_visible_child(&*imp.status_page_ask_confirmation);
                self.show_progress_indeterminate(false);
                self.enable_back_button(true);

                imp.status_page_ask_confirmation.set_description(Some(&gettextf(
                    // Translators: File receive confirmation message dialog; Filename, File size
                    "Your peer wants to send you “{0}” (Size: {1}).\nDo you want to download this file? The default action will save the file to your Downloads folder.",
                    &[&filename,
                        &glib::format_size(*size)]
                )));

                let notification = gio::Notification::new(&gettext("Ready to Receive File"));
                notification.set_body(Some(&gettext(
                    "A file is ready to be transferred. The transfer needs to be acknowledged.",
                )));
                notification.set_priority(gio::NotificationPriority::Urgent);
                notification.set_category(Some("transfer"));

                self.imp()
                    .send_notification_if_background(Some("receive-ready"), &notification);
            }
            UIState::Transmitting(filename, info) => {
                imp.stack.set_visible_child(&*imp.status_page_progress);
                self.show_progress_indeterminate(false);
                imp.progress_bar.set_show_text(true);
                self.enable_back_button(false);

                let mut ip = info.peer_addr.ip();
                // We convert ipv4 mapped ipv6 addresses because the gio code can't tell if they are
                // local or not
                if let IpAddr::V6(ipv6) = ip {
                    if let Some(ipv4) = ipv6.to_ipv4() {
                        ip = IpAddr::from(ipv4);
                    }
                }

                let gio_addr = gio::InetAddress::from(ip);
                let is_site_local = gio_addr.is_site_local();

                let description = match &info.conn_type {
                    wormhole::transit::ConnectionType::Direct => {
                        if is_site_local {
                            // Translators: Description, During transfer
                            gettextf("File “{}” via local network direct transfer", &[&filename])
                        } else {
                            // Translators: Description, During transfer
                            gettextf("File “{}” via direct transfer", &[&filename])
                        }
                    }
                    wormhole::transit::ConnectionType::Relay { name } => {
                        if let Some(name) = name {
                            // Translators: Description, During transfer
                            gettextf("File “{0}” via relay {1}", &[&filename, &name])
                        } else {
                            // Translators: Description, During transfer
                            gettextf("File “{}” via relay", &[&filename])
                        }
                    }
                    // Translators: Description, During transfer
                    _ => gettextf("File “{}” via Unknown connection method", &[&filename]),
                };

                imp.status_page_progress.set_description(Some(&description));

                if direction == TransferDirection::Send {
                    // Translators: Title
                    imp.status_page_progress.set_title(&gettext("Sending File"));
                    imp.status_page_progress
                        .set_icon_name(Some("send-to-symbolic"));
                } else {
                    // Translators: Title
                    imp.status_page_progress
                        .set_title(&gettext("Receiving File"));
                    imp.status_page_progress
                        .set_icon_name(Some("folder-download-symbolic"));
                }
            }
            UIState::Done(filename) => {
                imp.stack.set_visible_child(&*imp.status_page_success);
                self.enable_back_button(true);

                let notification = gio::Notification::new(&gettext("File Transfer Complete"));

                notification.set_priority(gio::NotificationPriority::High);
                notification.set_category(Some("transfer.complete"));

                if direction == TransferDirection::Send {
                    let description = gettextf(
                        // Translators: Description, Filename
                        "Successfully sent file “{}”",
                        &[&filename.to_string_lossy()],
                    );

                    imp.status_page_success.set_description(Some(&description));
                    notification.set_body(Some(&description));
                    imp.open_button.set_visible(false);
                    imp.open_dir_button.set_visible(false);
                } else {
                    match imp.context.borrow().file_path_received_successfully.clone() {
                        Some(path) => {
                            let description = super::fs::default_download_dir()
                                .ok()
                                .filter(|download_dir| path.parent() == Some(download_dir))
                                .map_or_else(
                                    || {
                                        gettextf(
                                            // Translators: Filename
                                            "File has been saved to the selected folder as “{}”",
                                            &[&filename.to_string_lossy()],
                                        )
                                    },
                                    |_dir| {
                                        gettextf(
                                            // Translators: Filename
                                            "File has been saved to the Downloads folder as “{}”",
                                            &[&filename.to_string_lossy()],
                                        )
                                    },
                                );

                            imp.status_page_success.set_description(Some(&description));
                            notification.set_body(Some(&description));

                            imp.open_button.set_visible(true);
                            imp.open_dir_button.set_visible(true);
                            notification.set_default_action_and_target_value(
                                crate::ui::window::Action::ShowFile.as_ref(),
                                Some(&path.to_variant()),
                            );
                        }
                        _ => {}
                    }
                }

                self.imp()
                    .send_notification_if_background(Some("transfer-complete"), &notification);
            }
            UIState::Error(error) => {
                imp.stack.set_visible_child(&*imp.status_page_error);
                imp.status_page_error
                    .set_description(Some(&error.gettext_error()));
                self.enable_back_button(true);

                let notification = gio::Notification::new(&gettext("File Transfer Failed"));
                notification.set_body(Some(&gettextf(
                    "The file transfer failed: {}",
                    &[&error.gettext_error()],
                )));
                notification.set_priority(gio::NotificationPriority::High);
                notification.set_category(Some("transfer.error"));

                self.imp()
                    .send_notification_if_background(Some("transfer-error"), &notification);

                let peer_canceled = if let AppError::Transfer {
                    source: wormhole::transfer::TransferError::PeerError(msg),
                } = error
                {
                    msg == "Task has been cancelled"
                } else {
                    false
                };

                if !error.is_user_canceled() && !peer_canceled {
                    imp.copy_error_button.set_visible(true);
                }
            }
        }
    }

    /// This will ask whether the transfer should be cancelled.
    pub async fn cancel_request(&self) -> bool {
        if matches!(
            &*self.imp().context.borrow().ui_state,
            UIState::AskConfirmation(..) | UIState::Done(..) | UIState::Error(..)
        ) {
            return true;
        }

        let response = self.window().ask_abort_dialog().choose_future(self).await;

        response == "abort"
    }

    async fn wait_for_cancellation_future(&self) {
        let mut channel = self
            .imp()
            .context
            .borrow()
            .cancellation_complete_receiver
            .clone();
        if let Err(err) = channel.recv().await {
            log::error!("Error when waiting for cancellation future {:?}", err);
        }
    }

    pub async fn cancel(&self) {
        log::info!("Cancelling transfer");
        self.imp().cancel_button.set_sensitive(false);
        self.imp().context.borrow_mut().canceled = true;

        let imp = self.imp();
        log::debug!("Sending cancel signal");
        let cancel_sender = imp.context.borrow().cancel_sender.clone();
        cancel_sender.broadcast(()).await.unwrap();
        self.wait_for_cancellation_future().await;
        self.window().navigate_home();
    }

    fn show_progress_indeterminate(&self, pulse: bool) {
        let imp = self.imp();
        if let Some(source_id) = imp.context.borrow_mut().progress_timeout_source_id.take() {
            source_id.remove();
        }

        if pulse {
            // 50 ms was mainly chosen for performance of the progress bar
            imp.context.borrow_mut().progress_timeout_source_id = Some(glib::timeout_add_local(
                Duration::from_millis(50),
                clone!(
                    #[strong(rename_to = obj)]
                    self,
                    move || {
                        obj.imp().progress_bar.pulse();

                        glib::ControlFlow::Continue
                    }
                ),
            ));
        }
    }

    async fn prepare_and_open_file(
        &self,
        path: &Path,
    ) -> Result<(smol::fs::File, PathBuf, OsString), AppError> {
        let mut filename = if let Some(filename) = path.file_name() {
            filename.to_os_string()
        } else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };

        let (file, path) = if path.is_dir() {
            self.set_ui_state(UIState::Archive(filename.clone()));
            filename.push(".zip");

            let temp_file = fs::compress_folder_cancelable(
                path,
                self.cancel_future(),
                Self::zip_progress_handler,
            )
            .await?;
            (
                smol::fs::File::from(temp_file.reopen()?),
                temp_file.path().to_path_buf(),
            )
        } else if path.is_file() {
            (
                smol::fs::OpenOptions::new().read(true).open(path).await?,
                path.to_path_buf(),
            )
        } else {
            // Translators: When opening a file
            return Err(UiError::new(&gettext("Specified file / directory does not exist")).into());
        };

        Ok((file, path, filename))
    }

    fn prepare_transmit(&self, direction: TransferDirection) -> Result<(), AppError> {
        self.reset();

        if let Some(app) = self.app() {
            app.inhibit_transfer(direction);
        }

        self.set_transfer_direction(direction);
        self.set_ui_state(UIState::Initial);

        let rendezvous_url = self
            .window()
            .config()
            .rendezvous_server_url()
            .map_err(|_| {
                UiError::new(&gettext(
                "Error parsing rendezvous server URL. An invalid URL was entered in the settings.",
            ))
            })?;
        self.imp().context.borrow_mut().rendezvous_url = rendezvous_url;

        let transit_url = self.window().config().transit_relay_hints().map_err(|_| {
            UiError::new(&gettext(
                "Error parsing transit URL. An invalid URL was entered in the settings.",
            ))
        })?;
        self.imp().context.borrow_mut().relay_hints = transit_url;

        self.window().show_action_view();
        Ok(())
    }

    async fn transmit_receive(
        &self,
        code: wormhole::Code,
        app_cfg: wormhole::AppConfig<wormhole::transfer::AppVersion>,
    ) -> Result<(), AppError> {
        self.prepare_transmit(TransferDirection::Receive)?;
        let uri = WormholeTransferURI::from_app_cfg_with_code_direction(
            &app_cfg,
            code.clone(),
            TransferDirection::Send,
        );
        self.set_ui_state(UIState::HasCode(uri));

        self.window().add_code(&code);

        let connection = Box::pin(spawn_async(cancelable_future(
            wormhole::MailboxConnection::connect(app_cfg, code, false),
            self.cancel_future(),
        )))
        .await??;

        let wormhole = Box::pin(spawn_async(cancelable_future(
            wormhole::Wormhole::connect(connection),
            self.cancel_future(),
        )))
        .await??;

        self.set_ui_state(UIState::Connected);

        let relay_url = self.imp().context.borrow().relay_hints.clone();

        let request = spawn_async(wormhole::transfer::request_file(
            wormhole,
            relay_url,
            TRANSIT_ABILITIES,
            self.cancel_future(),
        ))
        .await?
        .ok_or(AppError::Canceled)?;

        // Only use the last filename component otherwise the other side can overwrite
        // files in different directories
        let offer_filename = request.file_name();
        let offer_filename = if offer_filename.is_empty() {
            // This shouldn't happen realistically
            "Unknown Filename.bin".to_owned()
        } else {
            offer_filename
        };

        self.set_ui_state(UIState::AskConfirmation(
            offer_filename.clone(),
            request.file_size(),
        ));

        // Continue or cancel
        self.imp()
            .save_as_file_dialog
            .set_initial_name(Some(&offer_filename));
        let res = self.ask_confirmation_future().await;
        let selected_download_file_path = match res {
            Ok(selected_path) => selected_path,
            Err(err) => {
                smol::spawn(async move {
                    if let Err(err) = request.reject().await {
                        log::error!("An error occurred when rejecting a transfer: {}", err);
                    }
                })
                .await;

                return Err(err);
            }
        };

        let use_temp_path = selected_download_file_path.is_none();
        let download_file_path = selected_download_file_path
            .unwrap_or(super::fs::default_download_dir()?.join(offer_filename));

        self.set_ui_state(UIState::Connected);

        if let Some(app) = self.app() {
            app.withdraw_notification("receive-ready");
        }

        let download_file_name =
            PathBuf::from(download_file_path.file_name().ok_or_else(|| {
                UiError::new(&gettextf(
                    "Invalid path selected: {}",
                    &[&download_file_path.display()],
                ))
            })?);

        let temp_file = if use_temp_path {
            let mut tempfile_prefix = download_file_name.as_os_str().to_os_string();
            tempfile_prefix.push(".");

            tempfile::Builder::new()
                .prefix(&tempfile_prefix)
                .suffix(&".warpdownload")
                .tempfile_in(download_file_path.parent().unwrap_or(&PathBuf::from("./")))?
        } else {
            let file = std::fs::File::create(&download_file_path)?;
            tempfile::NamedTempFile::from_parts(
                file,
                tempfile::TempPath::from_path(&download_file_path),
            )
        };

        let async_file = smol::fs::File::from(temp_file.reopen()?);

        self.imp().context.borrow_mut().file_name =
            Some(download_file_name.as_os_str().to_os_string());

        log::info!(
            "Downloading file to {:?}",
            temp_file.path().to_string_lossy()
        );

        let file = cancelable_future(
            spawn_async(async move {
                let mut file = async_file;
                Box::pin(request.accept(
                    Self::transit_handler_main,
                    Self::progress_handler_main,
                    &mut file,
                    Self::cancel_future_main().await,
                ))
                .await?;
                AppError::ok(file)
            }),
            Self::cancel_timeout_future(TIMEOUT_MS),
        )
        .await??;

        if self.imp().context.borrow().canceled {
            return Err(AppError::Canceled);
        }

        // Windows requires the file to be closed before renaming it
        file.sync_all().await?;
        drop(file);

        // Rename the file to its final name
        let path = if use_temp_path {
            safe_persist_tempfile(temp_file, &download_file_name)?
        } else {
            temp_file.keep().map_err(|err| err.error)?.1
        };

        self.imp().context.borrow_mut().file_name = Some(path.file_name().unwrap().to_os_string());
        self.imp()
            .context
            .borrow_mut()
            .file_path_received_successfully = Some(path);
        self.transmit_success();

        Ok(())
    }

    async fn transmit_send(
        &self,
        path: PathBuf,
        app_cfg: wormhole::AppConfig<wormhole::transfer::AppVersion>,
    ) -> Result<(), AppError> {
        self.prepare_transmit(TransferDirection::Send)?;
        self.set_ui_state(UIState::RequestCode);

        let window = self.window();

        let (mut file, path, filename) = self.prepare_and_open_file(&path).await?;
        self.imp().context.borrow_mut().file_name = Some(filename.clone());
        let code_length = window.config().code_length_or_default();

        let connection = spawn_async(cancelable_future(
            wormhole::MailboxConnection::create(app_cfg.clone(), code_length),
            self.cancel_future(),
        ))
        .await??;

        window.add_code(connection.code());
        let uri = WormholeTransferURI::from_app_cfg_with_code_direction(
            &app_cfg,
            connection.code().to_owned(),
            TransferDirection::Receive,
        );
        self.set_ui_state(UIState::HasCode(uri));

        let wormhole = spawn_async(cancelable_future(
            wormhole::Wormhole::connect(connection),
            self.cancel_future(),
        ))
        .await??;

        self.set_ui_state(UIState::Connected);

        self.imp().context.borrow_mut().file_path = Some(path);
        let transit_url = self.imp().context.borrow().relay_hints.clone();

        let metadata = file.metadata().await?;

        cancelable_future(
            spawn_async(async move {
                Box::pin(wormhole::transfer::send_file(
                    wormhole,
                    transit_url,
                    &mut file,
                    filename.to_string_lossy(),
                    metadata.len(),
                    TRANSIT_ABILITIES,
                    Self::transit_handler_main,
                    Self::progress_handler_main,
                    Self::cancel_future_main().await,
                ))
                .await
            }),
            Self::cancel_timeout_future(TIMEOUT_MS),
        )
        .await??;

        if self.imp().context.borrow().canceled {
            return Err(AppError::Canceled);
        }

        self.transmit_success();

        Ok(())
    }

    /// Wrapper to handle waiting on a channel that receives ()
    async fn receiver_future(name: &str, mut receiver: async_broadcast::Receiver<()>) {
        let res = receiver.recv().await;
        match res {
            Ok(()) => {
                log::debug!("Receiver future '{name}' received signal");
            }
            Err(err) => {
                log::error!("Receiver future '{name}' received error: {err:?}");
            }
        }
    }

    /// This future will finish when a message is received in the cancellation channel
    async fn cancel_future_main() -> impl Future<Output = ()> {
        block_on_main_with_app_async(|app| app.main_window().action_view().cancel_future()).await
    }

    /// This future will finish when a message is received in the cancellation channel
    fn cancel_future(&self) -> impl Future<Output = ()> + use<> {
        let cancel_receiver = self.imp().context.borrow().cancel_receiver.clone();
        Self::receiver_future("cancel", cancel_receiver)
    }

    /// This future is for any wormhole calls that have proper cancellation but no timeout handling
    ///
    /// We will wait until a cancellation event is received. Then we give the wormhole code
    /// `timeout_ms` milliseconds to properly respond. When there is no response the future will
    /// be finished
    fn cancel_timeout_future(timeout_ms: u64) -> impl Future<Output = ()> {
        let (sender, receiver) = async_broadcast::broadcast(1);
        async move {
            // Wait for a cancellation event
            Self::cancel_future_main().await.await;

            // Then do a timeout
            glib::timeout_add_once(Duration::from_millis(timeout_ms), move || {
                log::debug!("Cancellation timeout");
                if let Err(err) = sender.try_broadcast(()) {
                    log::error!("Error when sending cancellation timeout message: {:?}", err);
                }
            });

            Self::receiver_future("timeout", receiver).await;
        }
    }

    /// Callback with information about the currently running transfer
    fn transit_handler_main(info: wormhole::transit::TransitInfo) {
        invoke_main_with_app(move |app| {
            let obj = app.main_window().action_view();
            let imp = obj.imp();

            let filename = imp
                .context
                .borrow()
                .file_name
                .as_ref()
                .map_or_else(|| "?".to_owned(), |s| s.to_string_lossy().to_string());

            obj.set_ui_state(UIState::Transmitting(filename, info));
        });
    }

    /// Handles progress information updates
    fn progress_handler_main(sent: u64, total: u64) {
        invoke_main_with_app(move |app| {
            let obj = app.main_window().action_view();
            let imp = obj.imp();

            if imp.context.borrow().progress.is_none() {
                imp.context.borrow_mut().progress =
                    Some(FileTransferProgress::begin(total as usize));
            }

            let mut update_progress = false;
            let Some((fraction, progress_str)) =
                imp.context.borrow_mut().progress.as_mut().map(|progress| {
                    update_progress = progress.set_progress(sent as usize);
                    (
                        progress.progress_fraction(),
                        progress.pretty_time_remaining(),
                    )
                })
            else {
                return;
            };

            if update_progress {
                imp.progress_bar.set_fraction(fraction);
                imp.progress_bar
                    .set_text(Some(&progress_str.unwrap_or_default()));
            }
        });
    }

    fn zip_progress_handler(count: usize, size: usize) {
        invoke_main_with_app(move |app| {
            let obj = app.main_window().action_view();
            let imp = obj.imp();

            let size_str = glib::format_size(size as u64);
            let data_str = ngettextf(
                // Translators: Above progress bar for creating an archive to send as a folder
                "{} File - Size: {}",
                "{} Files - Size: {}",
                count as u32,
                &[&count.to_string(), &size_str],
            );

            log::trace!("ZIP progress: {}", data_str);
            imp.progress_bar.set_text(Some(&data_str));
        });
    }

    async fn ask_confirmation_future(&self) -> Result<Option<PathBuf>, AppError> {
        let mut continue_receiver = self.imp().context.borrow().continue_receiver.clone();
        let result = cancelable_future(continue_receiver.recv(), self.cancel_future()).await??;
        Ok(result)
    }

    /// Any post-transfer cleanup operations that are shared between success and failure states
    pub fn transmit_cleanup(&self) {
        log::debug!("Transmit cleanup");

        if let Some(app) = self.app() {
            app.uninhibit_transfer();
        }

        if self.imp().context.borrow().canceled {
            // Send the cancellation complete message
            if let Err(err) = self
                .imp()
                .context
                .borrow()
                .cancellation_complete_sender
                .try_broadcast(())
            {
                log::error!("Error sending cancellation complete message: {:?}", err);
            }
        }
    }

    /// Resets the view to be ready for the next transfer
    pub fn reset(&self) {
        log::debug!("Reset");
        let imp = self.imp();
        self.show_progress_indeterminate(false);

        imp.context.replace(UIContext::default());

        // Deletes any temporary files if required
        imp.context.borrow_mut().file_path = None;
    }

    fn transmit_success(&self) {
        log::debug!("Transmit success");

        self.imp().progress_bar.set_fraction(1.0);

        let file_name = self
            .imp()
            .context
            .borrow()
            .file_name
            .clone()
            .unwrap_or_else(|| OsString::from("?"));

        self.set_ui_state(UIState::Done(file_name));

        self.transmit_cleanup();
    }

    pub fn transmit_error(&self, error: AppError) {
        if *self.ui_state() == UIState::Initial {
            log::debug!("Transmit error, handle with dialog");
            error.handle();
        } else if !error.is_user_canceled() {
            log::debug!("Transmit error, show error page");
            self.set_ui_state(UIState::Error(error));
        }

        self.transmit_cleanup();
    }

    pub fn transmit_error_handler_main(error: AppError) {
        invoke_main_with_app(|app| {
            app.main_window().action_view().transmit_error(error);
        });
    }

    pub fn send_file(
        &self,
        path: PathBuf,
        app_cfg: wormhole::AppConfig<wormhole::transfer::AppVersion>,
    ) {
        log::info!("Sending file: {}", path.display());
        let obj = self.clone();

        main_async_local(Self::transmit_error_handler_main, async move {
            Box::pin(obj.transmit_send(path, app_cfg)).await?;
            Ok(())
        });
    }

    pub fn receive_file(
        &self,
        code: wormhole::Code,
        app_cfg: wormhole::AppConfig<wormhole::transfer::AppVersion>,
    ) {
        log::info!("Receiving file with code '{}'", code);
        let obj = self.clone();

        main_async_local(Self::transmit_error_handler_main, async move {
            Box::pin(obj.transmit_receive(code, app_cfg)).await?;
            Ok(())
        });
    }

    pub fn transfer_in_progress(&self) -> bool {
        !self.imp().context.borrow().canceled
            && !matches!(
                &*self.ui_state(),
                UIState::Initial | UIState::Done(..) | UIState::Error(..)
            )
    }

    pub fn should_handle_error_inline(&self) -> bool {
        !matches!(
            &*self.ui_state(),
            UIState::Initial | UIState::Done(..) | UIState::Error(..)
        )
    }
}
