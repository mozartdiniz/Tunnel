/// GObject implementation of the main application window.
///
/// All persistent widget state lives here in `RefCell` / `OnceCell` fields.
/// Signal handlers are wired in `constructed()` using `glib::clone!(#[weak])` to
/// prevent reference cycles.
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;

use gtk4::glib;
use gtk4::glib::subclass::InitializingObject;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::CompositeTemplate;
use libadwaita::subclass::prelude::*;

use crate::app::AppCommand;
use crate::config::Config;

#[derive(Default, CompositeTemplate)]
#[template(resource = "/dev/tunnel/Tunnel/window.ui")]
pub struct Window {
    // ── Template children (bound from window.ui) ─────────────────────────────
    #[template_child]
    pub list_box: TemplateChild<gtk4::ListBox>,
    #[template_child]
    pub stack: TemplateChild<gtk4::Stack>,
    #[template_child]
    pub window_title: TemplateChild<libadwaita::WindowTitle>,
    #[template_child]
    pub refresh_btn: TemplateChild<gtk4::Button>,
    #[template_child]
    pub settings_btn: TemplateChild<gtk4::Button>,
    #[template_child]
    pub toast_overlay: TemplateChild<libadwaita::ToastOverlay>,

    // ── Runtime state (set once during setup, read-only thereafter) ───────────
    /// Command sender — wired up in `Window::setup()`, used by signal handlers.
    pub cmd_tx: std::cell::OnceCell<async_channel::Sender<AppCommand>>,
    /// Shared config — also held by the preferences dialog via Rc clone.
    pub config: std::cell::OnceCell<Rc<RefCell<Config>>>,

    // ── Mutable UI state ─────────────────────────────────────────────────────
    /// Live peer map: fingerprint → (display name, socket address).
    pub peers: RefCell<HashMap<String, (String, SocketAddr)>>,
    /// Pending idle-reset timers for sync peer rows, keyed by peer fingerprint.
    /// Cancelled and rescheduled on each sync TransferComplete so the row only
    /// resets after the last file in a batch finishes.
    pub sync_reset_source: RefCell<HashMap<String, glib::SourceId>>,
}

#[glib::object_subclass]
impl ObjectSubclass for Window {
    const NAME: &'static str = "TunnelWindow";
    type Type = super::Window;
    type ParentType = libadwaita::ApplicationWindow;

    fn class_init(klass: &mut Self::Class) {
        klass.bind_template();
    }

    fn instance_init(obj: &InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for Window {
    fn constructed(&self) {
        self.parent_constructed();

        let obj = self.obj();

        // ── Refresh button ────────────────────────────────────────────────────
        self.refresh_btn.connect_clicked(glib::clone!(#[weak] obj, move |_| {
            let imp = obj.imp();
            while let Some(child) = imp.list_box.first_child() {
                imp.list_box.remove(&child);
            }
            imp.peers.borrow_mut().clear();
            crate::ui::update_stack(&imp.list_box, &imp.stack);
            if let Some(tx) = imp.cmd_tx.get() {
                let _ = tx.try_send(AppCommand::RefreshPeers);
            }
        }));

        // ── Settings button ───────────────────────────────────────────────────
        self.settings_btn.connect_clicked(glib::clone!(#[weak] obj, move |_| {
            let imp = obj.imp();
            let Some(config) = imp.config.get() else { return };
            let Some(cmd_tx) = imp.cmd_tx.get() else { return };
            let window_title = (*imp.window_title).clone();
            crate::ui::show_preferences(
                obj.upcast_ref::<libadwaita::ApplicationWindow>(),
                config.clone(),
                cmd_tx.clone(),
                window_title,
            );
        }));
    }
}

impl WidgetImpl for Window {}
impl WindowImpl for Window {}
impl ApplicationWindowImpl for Window {}
impl AdwApplicationWindowImpl for Window {}
