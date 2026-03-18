use adw::prelude::*;
use adw::subclass::prelude::*;

use super::application::WarpApplication;

const CODE_LENGTH_MIN: i32 = 2;
const CODE_LENGTH_MAX: i32 = 8;

mod imp {
    use super::*;
    use crate::gettext::gettextf;
    use crate::globals;
    use glib::Properties;
    use std::cell::{Cell, RefCell};

    #[derive(Properties, Debug, Default, gtk::CompositeTemplate)]
    #[properties(wrapper_type = super::WarpPreferencesDialog)]
    #[template(file = "preferences.ui")]
    pub struct WarpPreferencesDialog {
        #[template_child]
        pub preferences_group: TemplateChild<adw::PreferencesGroup>,

        #[template_child]
        pub rendezvous_server_url_entry_row: TemplateChild<adw::EntryRow>,

        #[template_child]
        pub transit_server_url_entry_row: TemplateChild<adw::EntryRow>,

        #[template_child]
        pub code_length_spin_row: TemplateChild<adw::SpinRow>,

        #[property(get, set = Self::set_rendezvous_server_url)]
        pub rendezvous_server_url: RefCell<String>,
        #[property(get, set = Self::set_transit_server_url)]
        pub transit_server_url: RefCell<String>,
        #[property(get, set, default = 4, minimum = CODE_LENGTH_MIN, maximum = CODE_LENGTH_MAX)]
        pub code_length: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WarpPreferencesDialog {
        const NAME: &'static str = "WarpPreferencesDialog";
        type Type = super::WarpPreferencesDialog;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        // You must call `Widget`'s `init_template()` within `instance_init()`.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for WarpPreferencesDialog {
        fn properties() -> &'static [glib::ParamSpec] {
            Self::derived_properties()
        }

        fn set_property(&self, id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            self.derived_set_property(id, value, pspec);
        }

        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            self.derived_property(id, pspec)
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let window = self.app().main_window();
            obj.set_rendezvous_server_url(
                window
                    .config()
                    .rendezvous_server_url
                    .clone()
                    .unwrap_or_default(),
            );
            obj.set_transit_server_url(
                window
                    .config()
                    .transit_server_url
                    .clone()
                    .unwrap_or_default(),
            );

            self.code_length_spin_row
                .set_adjustment(Some(&gtk::Adjustment::new(
                    4f64, 2f64, 8f64, 1f64, 0f64, 0f64,
                )));

            obj.set_code_length(window.config().code_length_or_default() as i32);

            self.preferences_group.set_description(Some(&gettextf(
                "Changing the rendezvous server URL needs to be done on both sides of the \
transfer. Only enter a server URL you can trust.\n\
\n\
Leaving these entries empty will use the app defaults:\n\
Rendezvous Server: “{0}”\n\
Transit Server: “{1}”",
                &[
                    &globals::WORMHOLE_DEFAULT_RENDEZVOUS_SERVER_STR,
                    &globals::WORMHOLE_DEFAULT_TRANSIT_RELAY_URL_STR,
                ],
            )));
        }
    }

    impl WidgetImpl for WarpPreferencesDialog {}

    impl AdwDialogImpl for WarpPreferencesDialog {
        fn closed(&self) {
            let window = self.app().main_window();

            let rendezvous_url = &*self.rendezvous_server_url.borrow();
            window.config().rendezvous_server_url =
                (!rendezvous_url.is_empty()).then(|| rendezvous_url.clone());

            let transit_url = &*self.transit_server_url.borrow();
            window.config().transit_server_url =
                (!transit_url.is_empty()).then(|| transit_url.clone());

            let code_length = self.code_length.get();
            window.config().code_length = Some(code_length as usize);

            window.save_config();
        }
    }
    impl PreferencesDialogImpl for WarpPreferencesDialog {}

    impl WarpPreferencesDialog {
        fn app(&self) -> WarpApplication {
            WarpApplication::default()
        }

        fn set_rendezvous_server_url(&self, url: String) {
            let is_valid_url = url::Url::parse(&url).is_ok();

            if is_valid_url && !url.is_empty() {
                self.rendezvous_server_url_entry_row
                    .add_css_class("success");
            } else {
                self.rendezvous_server_url_entry_row
                    .remove_css_class("success");
            }

            if url.is_empty() || is_valid_url {
                self.rendezvous_server_url_entry_row
                    .remove_css_class("error");
                self.rendezvous_server_url.replace(url);
            } else {
                self.rendezvous_server_url_entry_row.add_css_class("error");
                self.rendezvous_server_url.replace(String::new());
            }
        }

        fn set_transit_server_url(&self, url: String) {
            let url_res = url::Url::parse(&url);
            let is_valid_hint = if let Ok(url) = url_res {
                wormhole::transit::RelayHint::from_urls(None, [url]).is_ok()
            } else {
                false
            };

            if is_valid_hint && !url.is_empty() {
                self.transit_server_url_entry_row.add_css_class("success");
            } else {
                self.transit_server_url_entry_row
                    .remove_css_class("success");
            }

            if url.is_empty() || is_valid_hint {
                self.transit_server_url_entry_row.remove_css_class("error");
                self.transit_server_url.replace(url);
            } else {
                self.transit_server_url_entry_row.add_css_class("error");
                self.transit_server_url.replace(String::new());
            }
        }
    }
}

glib::wrapper! {
    pub struct WarpPreferencesDialog(ObjectSubclass<imp::WarpPreferencesDialog>)
        @extends gtk::Widget, adw::Dialog, adw::PreferencesDialog,
        @implements gtk::Accessible, gtk::ConstraintTarget, gtk::Buildable;
}

impl WarpPreferencesDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for WarpPreferencesDialog {
    fn default() -> Self {
        Self::new()
    }
}
