mod dialogs;
mod event_handler;
mod helpers;
mod notifications;
mod peer_list;
mod preferences;

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;

use async_channel::{Receiver, Sender};
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::app::{AppCommand, AppEvent};
use crate::config::Config;

use self::event_handler::handle_event;
use self::peer_list::update_stack;
use self::preferences::show_preferences;

pub fn build_ui(
    app: &libadwaita::Application,
    config: Config,
    event_rx: Receiver<AppEvent>,
    cmd_tx: Sender<AppCommand>,
) {
    // ── Register notification actions on the application ─────────────────────
    // These allow notification buttons (Accept/Deny) to work even when the
    // window is in the background.
    {
        let cmd_tx_a = cmd_tx.clone();
        let accept = gio::SimpleAction::new(
            "accept-transfer",
            Some(&String::static_variant_type()),
        );
        accept.connect_activate(move |_, param| {
            if let Some(id) = param.and_then(|v| v.get::<String>()) {
                let _ = cmd_tx_a.send_blocking(AppCommand::AcceptTransfer { transfer_id: id });
            }
        });
        app.add_action(&accept);
    }
    {
        let cmd_tx_d = cmd_tx.clone();
        let deny = gio::SimpleAction::new(
            "deny-transfer",
            Some(&String::static_variant_type()),
        );
        deny.connect_activate(move |_, param| {
            if let Some(id) = param.and_then(|v| v.get::<String>()) {
                let _ = cmd_tx_d.send_blocking(AppCommand::DenyTransfer { transfer_id: id });
            }
        });
        app.add_action(&deny);
    }
    {
        let reveal = gio::SimpleAction::new(
            "reveal-file",
            Some(&String::static_variant_type()),
        );
        reveal.connect_activate(move |_, param| {
            if let Some(path_str) = param.and_then(|v| v.get::<String>()) {
                let file = gio::File::for_path(&path_str);
                // For a directory `saved_to` open it directly; for a file open its parent.
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

    let window = build_main_window(app, Rc::new(RefCell::new(config)), event_rx, cmd_tx);
    window.present();
}

fn build_main_window(
    app: &libadwaita::Application,
    config: Rc<RefCell<Config>>,
    event_rx: Receiver<AppEvent>,
    cmd_tx: Sender<AppCommand>,
) -> libadwaita::ApplicationWindow {
    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Tunnel")
        .default_width(420)
        .default_height(560)
        .icon_name("dev.tunnel.Tunnel")
        .build();

    let toolbar_view = libadwaita::ToolbarView::new();
    window.set_content(Some(&toolbar_view));

    // ── Load CSS from GResource ───────────────────────────────────────────────
    let css = gtk4::CssProvider::new();
    css.load_from_resource("/dev/tunnel/Tunnel/style.css");
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk4::IconTheme::for_display(&display).add_resource_path("/dev/tunnel/Tunnel/icons");
    }

    // ── Header bar ────────────────────────────────────────────────────────────
    let header = libadwaita::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let window_title = libadwaita::WindowTitle::builder()
        .title("Tunnel")
        .subtitle(&config.borrow().device_name)
        .build();
    header.set_title_widget(Some(&window_title));

    let refresh_btn = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh peer list")
        .build();
    header.pack_start(&refresh_btn);

    let settings_btn = gtk4::Button::builder()
        .icon_name("preferences-system-symbolic")
        .tooltip_text("Settings")
        .build();
    header.pack_end(&settings_btn);

    // ── Main content ──────────────────────────────────────────────────────────
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(0)
        .build();
    toolbar_view.set_content(Some(&content));

    let list_box = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(24)
        .margin_bottom(12)
        .margin_start(24)
        .margin_end(24)
        .build();

    let scrolled = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .child(&list_box)
        .build();

    let empty_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(12)
        .vexpand(true)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::Center)
        .build();

    let search_icon = gtk4::Image::builder()
        .icon_name("search-spinner-symbolic")
        .pixel_size(96)
        .build();
    search_icon.add_css_class("dim-label");
    search_icon.add_css_class("spinning-icon");

    let empty_title = gtk4::Label::builder()
        .label("Searching…")
        .css_classes(["title-2"])
        .margin_top(8)
        .build();

    let empty_label = gtk4::Label::builder()
        .label("Looking for devices on your network")
        .css_classes(["dim-label"])
        .build();

    empty_box.append(&search_icon);
    empty_box.append(&empty_title);
    empty_box.append(&empty_label);

    let stack = gtk4::Stack::new();
    stack.add_named(&empty_box, Some("empty"));
    stack.add_named(&scrolled, Some("list"));
    stack.set_visible_child_name("empty");
    content.append(&stack);

    // ── Status dot — CSS classes map to Adwaita named colour tokens ──────────
    let status_dot = gtk4::Label::builder()
        .label("●")
        .halign(gtk4::Align::Center)
        .margin_top(14)
        .margin_bottom(14)
        .build();
    status_dot.add_css_class("status-dot-idle");
    content.append(&status_dot);

    // ── Progress bar — hidden until a transfer starts ─────────────────────────
    let progress_bar = gtk4::ProgressBar::builder()
        .show_text(true)
        .margin_start(24)
        .margin_end(24)
        .margin_bottom(8)
        .visible(false)
        .build();
    toolbar_view.add_bottom_bar(&progress_bar);

    // ── Settings button ───────────────────────────────────────────────────────
    {
        let config = config.clone();
        let cmd_tx = cmd_tx.clone();
        let window = window.clone();
        let window_title = window_title.clone();
        settings_btn.connect_clicked(move |_| {
            show_preferences(&window, config.clone(), cmd_tx.clone(), window_title.clone());
        });
    }

    // ── Peer tracking ─────────────────────────────────────────────────────────
    let peers: Rc<RefCell<HashMap<String, (String, SocketAddr)>>> =
        Rc::new(RefCell::new(HashMap::new()));

    // ── Refresh button ────────────────────────────────────────────────────────
    {
        let list_box = list_box.clone();
        let peers = peers.clone();
        let stack = stack.clone();
        let cmd_tx = cmd_tx.clone();
        refresh_btn.connect_clicked(move |_| {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            peers.borrow_mut().clear();
            update_stack(&list_box, &stack);
            let _ = cmd_tx.send_blocking(AppCommand::RefreshPeers);
        });
    }

    // ── Event loop ────────────────────────────────────────────────────────────
    let list_box_c = list_box.clone();
    let stack_c = stack.clone();
    let status_dot_c = status_dot.clone();
    let progress_bar_c = progress_bar.clone();
    let peers_c = peers.clone();
    let cmd_tx_c = cmd_tx.clone();
    let window_c = window.clone();

    glib::MainContext::default().spawn_local(async move {
        while let Ok(event) = event_rx.recv().await {
            handle_event(
                event,
                &list_box_c,
                &stack_c,
                &status_dot_c,
                &progress_bar_c,
                &peers_c,
                &cmd_tx_c,
                &window_c,
            );
        }
    });

    window
}

