use crate::globals;
use crate::ui::application::WarpApplication;
use adw::prelude::*;
use adw::subclass::prelude::*;

mod imp {
    use super::*;
    use gtk::CompositeTemplate;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(file = "welcome_dialog.ui")]
    pub struct WelcomeDialog {
        #[template_child]
        pub(super) navigation_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub(super) page_detail: TemplateChild<adw::NavigationPage>,
        #[template_child]
        status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        next_button: TemplateChild<gtk::Button>,
        #[template_child]
        get_started_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WelcomeDialog {
        const NAME: &'static str = "WelcomeDialog";
        type Type = super::WelcomeDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_instance_callbacks();
        }

        // You must call `Widget`'s `init_template()` within `instance_init()`.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for WelcomeDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.status_page.set_icon_name(Some(globals::APP_ID));
        }
    }

    impl WidgetImpl for WelcomeDialog {}
    impl AdwDialogImpl for WelcomeDialog {}
}

glib::wrapper! {
    pub struct WelcomeDialog(ObjectSubclass<imp::WelcomeDialog>)
        @extends gtk::Widget, adw::Dialog,
        @implements gtk::Accessible, gtk::ConstraintTarget, gtk::Buildable;
}

#[gtk::template_callbacks]
impl WelcomeDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn app(&self) -> WarpApplication {
        WarpApplication::default()
    }

    #[template_callback]
    pub fn push_detail(&self) {
        self.imp().navigation_view.push(&*self.imp().page_detail);
    }

    #[template_callback]
    pub fn get_started_button_clicked(&self) {
        self.app().main_window().set_welcome_window_shown(true);
        self.close();
    }
}

impl Default for WelcomeDialog {
    fn default() -> Self {
        Self::new()
    }
}
