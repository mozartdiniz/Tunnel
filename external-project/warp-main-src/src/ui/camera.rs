use std::os::fd::OwnedFd;
use std::pin::pin;
use std::sync::Once;

use adw::prelude::*;
use adw::subclass::prelude::*;
use futures::FutureExt;
use gtk::glib::{self, clone};
use std::sync::LazyLock;

use crate::gettext::*;
use crate::ui::camera_row::CameraRow;
use crate::util::error::*;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use glib::subclass::{InitializingObject, Signal};

    use crate::{globals, ui::window::WarpApplicationWindow, util::future::main_async_local};

    use super::*;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(file = "camera.ui")]
    pub struct Camera {
        #[template_child]
        pub toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub viewfinder_bin: TemplateChild<adw::Bin>,
        #[template_child]
        pub selection_button: TemplateChild<gtk::MenuButton>,
        pub selection: gtk::SingleSelection,
        pub viewfinder: OnceCell<aperture::Viewfinder>,

        pub portal_cancellable: RefCell<Option<gio::Cancellable>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Camera {
        const NAME: &'static str = "Camera";
        type Type = super::Camera;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("camera");
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Camera {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("code-detected")
                        .param_types([glib::Bytes::static_type()])
                        .run_first()
                        .build(),
                ]
            });
            SIGNALS.as_ref()
        }
    }

    impl WidgetImpl for Camera {}
    impl NavigationPageImpl for Camera {
        fn hidden(&self) {
            if let Some(viewfinder) = self.viewfinder.get() {
                viewfinder.stop_stream();
            }
        }

        fn showing(&self) {
            log::debug!("Showing camera");
            self.init();

            let camera = self.obj();
            main_async_local(
                glib::clone!(
                    #[weak]
                    camera,
                    move |err| camera.imp().handle_error(&err)
                ),
                glib::clone!(
                    #[strong]
                    camera,
                    async move { camera.scan_from_camera().await }
                ),
            );
        }
    }

    #[gtk::template_callbacks]
    impl Camera {
        pub(super) fn init(&self) {
            static INIT: Once = Once::new();

            let obj = self.obj();

            INIT.call_once(move || {
                log::debug!("Initializing aperture");
                aperture::init(globals::APP_ID);

                log::debug!("Creating viewfinder");
                let viewfinder = self.viewfinder.get_or_init(aperture::Viewfinder::new);
                viewfinder.set_detect_codes(true);

                viewfinder.connect_state_notify(glib::clone!(
                    #[weak]
                    obj,
                    move |viewfinder| {
                        obj.update_viewfinder_state(viewfinder);
                    }
                ));
                obj.update_viewfinder_state(viewfinder);

                viewfinder.connect_code_detected(glib::clone!(
                    #[weak]
                    obj,
                    move |_, code| {
                        obj.emit_by_name::<()>("code-detected", &[&code]);
                    }
                ));

                let popover = gtk::Popover::new();
                popover.add_css_class("menu");

                log::debug!("Creating device provider");
                self.selection
                    .set_model(Some(aperture::DeviceProvider::instance()));
                let factory = gtk::SignalListItemFactory::new();
                factory.connect_setup(|_, item| {
                    let camera_row = CameraRow::default();

                    item.downcast_ref::<gtk::ListItem>()
                        .unwrap()
                        .set_child(Some(&camera_row));
                });
                let selection = &self.selection;
                factory.connect_bind(glib::clone!(
                    #[weak]
                    selection,
                    move |_, item| {
                        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                        let child = item.child().unwrap();
                        let row = child.downcast_ref::<CameraRow>().unwrap();

                        let item = item.item().and_downcast::<aperture::Camera>().unwrap();
                        row.set_label(&item.display_name());

                        selection.connect_selected_item_notify(glib::clone!(
                            #[weak]
                            row,
                            #[weak]
                            item,
                            move |selection| {
                                match selection.selected_item() {
                                    Some(selected_item) => {
                                        row.set_selected(selected_item == item);
                                    }
                                    _ => {
                                        row.set_selected(false);
                                    }
                                }
                            }
                        ));
                    }
                ));
                let list_view = gtk::ListView::new(Some(self.selection.clone()), Some(factory));
                popover.set_child(Some(&list_view));

                self.selection.connect_selected_item_notify(glib::clone!(
                    #[weak]
                    obj,
                    #[weak]
                    popover,
                    move |selection| {
                        popover.popdown();

                        let Some(viewfinder) = obj.imp().viewfinder.get() else {
                            return;
                        };

                        let camera = selection.selected_item().and_downcast::<aperture::Camera>();
                        if matches!(
                            viewfinder.state(),
                            aperture::ViewfinderState::Ready | aperture::ViewfinderState::Error
                        ) {
                            viewfinder.set_camera(camera);
                        }
                    }
                ));

                self.selection_button.set_popover(Some(&popover));

                log::debug!("Showing camera widget");
                self.viewfinder_bin.set_child(Some(viewfinder));

                log::debug!("Camera init done");
            });
        }

        #[template_callback]
        fn on_retry_clicked(&self) {
            let camera = self.obj();
            main_async_local(
                glib::clone!(
                    #[weak]
                    camera,
                    move |err| camera.imp().handle_error(&err)
                ),
                glib::clone!(
                    #[strong]
                    camera,
                    async move { camera.scan_from_camera().await }
                ),
            );
        }

        #[template_callback]
        fn on_troubleshooting_clicked(&self) {
            if let Some(app) = self
                .obj()
                .root()
                .and_downcast::<WarpApplicationWindow>()
                .and_then(|window| window.app())
            {
                app.open_help(Some("details-qr"));
            }
        }

        fn handle_error(&self, error: &AppError) {
            log::error!("Camera error: {}", error);
            let description = if let AppError::Ashpd {
                source: ashpd::Error::Portal(ashpd::PortalError::NotAllowed(..)),
            } = error
            {
                gettextf(
                    "Camera access denied. Open Settings and allow Warp to access the camera.",
                    &[&error.gettext_error()],
                )
            } else {
                gettextf("Failed to start the camera: {}", &[&error.gettext_error()])
            };

            self.error_page.set_title(&gettext("Error"));
            self.error_page.set_description(Some(&description));
            self.stack.set_visible_child_name("error");
        }

        pub(super) async fn request_permission(&self) -> Result<OwnedFd, AppError> {
            log::debug!("Requesting access to the camera");

            if let Some(cancellable) = self.portal_cancellable.take() {
                log::debug!("Canceling last operation");
                cancellable.cancel();
            }

            let cancellable = gio::Cancellable::new();
            self.portal_cancellable.replace(Some(cancellable.clone()));

            let proxy = ashpd::desktop::camera::Camera::new().await?;

            let cancel_fut = pin!(cancellable.future());
            let mut cancel = cancel_fut.fuse();
            let request = pin!(proxy.request_access());
            let mut access_request = request.fuse();
            futures::select! {
                () = cancel => {
                    log::debug!("Canceled");
                    self.portal_cancellable.take();
                    Err(AppError::Canceled)
                },
                res = access_request => Ok(res?)
            }?;

            self.portal_cancellable.take();

            log::debug!("Open PipeWire remote");
            Ok(proxy.open_pipe_wire_remote().await?)
        }
    }
}

glib::wrapper! {
    pub struct Camera(ObjectSubclass<imp::Camera>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::ConstraintTarget, gtk::Buildable;
}

impl Camera {
    pub fn connect_code_detected<F>(&self, callback: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, glib::Bytes) + 'static,
    {
        self.connect_local(
            "code-detected",
            false,
            clone!(
                #[weak(rename_to = camera)]
                self,
                #[upgrade_or_default]
                move |args| {
                    let code = args[1].get::<glib::Bytes>().unwrap();
                    callback(&camera, code);
                    None
                }
            ),
        )
    }

    async fn scan_from_camera(&self) -> Result<(), AppError> {
        static INIT: Once = Once::new();

        log::debug!("Starting camera stream");
        if let Some(viewfinder) = self.imp().viewfinder.get() {
            viewfinder.start_stream();
        } else {
            self.imp().init();
        }

        if INIT.is_completed() {
            return Ok(());
        }

        log::debug!("Initializing camera");
        let obj = self.clone();
        let provider = aperture::DeviceProvider::instance();
        match glib::MainContext::default()
            .spawn_local(async move { obj.imp().request_permission().await })
            .await
        {
            Ok(Ok(fd)) => {
                if let Err(err) = provider.set_fd(fd) {
                    Err(
                        UiError::new(&gettextf("Could not use the camera portal: {}", &[&err]))
                            .into(),
                    )
                } else if let Err(err) = provider.start_with_default(|camera| {
                    matches!(camera.location(), aperture::CameraLocation::Back)
                }) {
                    Err(UiError::new(&gettextf(
                        "Could not start the device provider: {}",
                        &[&err],
                    ))
                    .into())
                } else {
                    log::debug!("Device provider started");
                    INIT.call_once(|| ());
                    Ok(())
                }
            }
            Ok(Err(err)) => Err(err),
            Err(glib::JoinError { .. }) => Ok(()),
        }
    }

    pub fn stop(&self) {
        log::debug!("Stopping camera stream");
        if let Some(viewfinder) = self.imp().viewfinder.get() {
            viewfinder.stop_stream();
        }
    }

    fn update_viewfinder_state(&self, viewfinder: &aperture::Viewfinder) {
        let imp = self.imp();
        let state = viewfinder.state();
        match state {
            aperture::ViewfinderState::Loading => {
                imp.stack.set_visible_child_name("loading");
            }
            aperture::ViewfinderState::Error | aperture::ViewfinderState::NoCameras => {
                imp.stack.set_visible_child_name("error");
                imp.error_page.set_title(&gettext("No Camera Found"));
                imp.error_page
                    .set_description(Some(&gettext("Connect a camera to scan QR codes")));
            }
            aperture::ViewfinderState::Ready => {
                imp.stack.set_visible_child_name("stream");
            }
        }

        log::info!("The camera state changed: {state:?}");

        let is_ready = matches!(state, aperture::ViewfinderState::Ready);
        self.imp()
            .toolbar_view
            .set_extend_content_to_top_edge(is_ready);
        if is_ready {
            self.imp().toolbar_view.add_css_class("extended");
        } else {
            self.imp().toolbar_view.remove_css_class("extended");
        }
    }
}

impl Default for Camera {
    fn default() -> Self {
        glib::Object::new()
    }
}
