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

fn handle_event(
    event: AppEvent,
    list_box: &gtk4::ListBox,
    stack: &gtk4::Stack,
    status_dot: &gtk4::Label,
    progress_bar: &gtk4::ProgressBar,
    peers: &Rc<RefCell<HashMap<String, (String, SocketAddr)>>>,
    cmd_tx: &Sender<AppCommand>,
    window: &libadwaita::ApplicationWindow,
) {
    match event {
        AppEvent::PeerFound { id, name, addr } => {
            peers.borrow_mut().insert(id.clone(), (name.clone(), addr));
            add_peer_row(list_box, &id, &name, addr, cmd_tx);
            update_stack(list_box, stack);
        }

        AppEvent::PeerLost { id } => {
            peers.borrow_mut().remove(&id);
            remove_peer_row(list_box, &id);
            update_stack(list_box, stack);
        }

        AppEvent::IncomingRequest {
            transfer_id,
            sender_name,
            file_name,
            file_count,
            size_bytes,
            peer_fingerprint,
        } => {
            show_transfer_request(
                window,
                transfer_id.clone(),
                sender_name.clone(),
                file_name.clone(),
                file_count,
                size_bytes,
                peer_fingerprint,
                cmd_tx,
            );
            send_incoming_notification(&transfer_id, &sender_name, &file_name, file_count, size_bytes);
        }

        AppEvent::TransferProgress {
            bytes_done,
            total_bytes,
            bytes_per_sec,
            eta_secs,
            ..
        } => {
            let fraction = if total_bytes > 0 {
                (bytes_done as f64 / total_bytes as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            progress_bar.set_fraction(fraction);

            let speed_str = if bytes_per_sec > 0 {
                format!("  {}ps", human_bytes(bytes_per_sec))
            } else {
                String::new()
            };
            let eta_str = match eta_secs {
                Some(s) if s < 3600 => format!("  ETA {}", format_eta(s)),
                Some(s) => format!("  ETA {}", format_eta(s)),
                None => String::new(),
            };
            progress_bar.set_text(Some(&format!(
                "{} / {}  ({:.1}%){}{}",
                human_bytes(bytes_done),
                human_bytes(total_bytes),
                fraction * 100.0,
                speed_str,
                eta_str,
            )));
            progress_bar.set_visible(true);
            set_status(status_dot, "transfer");
        }

        AppEvent::TransferComplete { saved_to, .. } => {
            progress_bar.set_fraction(1.0);
            let pb = progress_bar.clone();
            let sd = status_dot.clone();
            send_complete_notification(&saved_to);
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                pb.set_visible(false);
                pb.set_fraction(0.0);
                set_status(&sd, "idle");
            });
        }

        AppEvent::TransferError { message, .. } => {
            progress_bar.set_visible(false);
            progress_bar.set_fraction(0.0);
            set_status(status_dot, "error");
            tracing::warn!("Transfer failed: {message}");
        }
    }
}

// ── Status dot helpers ────────────────────────────────────────────────────────

fn set_status(dot: &gtk4::Label, state: &str) {
    dot.remove_css_class("status-dot-idle");
    dot.remove_css_class("status-dot-transfer");
    dot.remove_css_class("status-dot-error");
    dot.add_css_class(&format!("status-dot-{state}"));
}

// ── Desktop notifications (roadmap 4.1) ──────────────────────────────────────

fn send_incoming_notification(
    transfer_id: &str,
    sender_name: &str,
    file_name: &str,
    file_count: usize,
    size_bytes: u64,
) {
    let Some(app) = gio::Application::default() else { return };

    let what = if file_count == 1 {
        file_name.to_string()
    } else {
        format!("{file_count} files")
    };

    let n = gio::Notification::new("Incoming file");
    n.set_body(Some(&format!(
        "{sender_name} wants to send you {what} ({})",
        human_bytes(size_bytes)
    )));
    n.set_default_action("app.focus");
    n.add_button_with_target_value(
        "Accept",
        "app.accept-transfer",
        Some(&transfer_id.to_variant()),
    );
    n.add_button_with_target_value(
        "Deny",
        "app.deny-transfer",
        Some(&transfer_id.to_variant()),
    );
    app.send_notification(Some(transfer_id), &n);
}

fn send_complete_notification(saved_to: &std::path::PathBuf) {
    let Some(app) = gio::Application::default() else { return };

    let label = if saved_to.is_dir() {
        saved_to
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Downloads")
            .to_string()
    } else {
        saved_to
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Downloads")
            .to_string()
    };

    let n = gio::Notification::new("Transfer complete");
    n.set_body(Some(&format!("Saved to {label}")));
    n.add_button_with_target_value(
        "Open folder",
        "app.reveal-file",
        Some(&saved_to.to_string_lossy().to_variant()),
    );
    app.send_notification(None, &n);
}

// ── Preferences window ────────────────────────────────────────────────────────

fn show_preferences(
    parent: &libadwaita::ApplicationWindow,
    config: Rc<RefCell<Config>>,
    cmd_tx: Sender<AppCommand>,
    window_title: libadwaita::WindowTitle,
) {
    let prefs = libadwaita::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .title("Settings")
        .build();

    let page = libadwaita::PreferencesPage::new();

    let device_group = libadwaita::PreferencesGroup::builder()
        .title("Device")
        .build();

    let name_row = libadwaita::EntryRow::builder()
        .title("Device Name")
        .text(&config.borrow().device_name)
        .build();
    device_group.add(&name_row);
    page.add(&device_group);

    let transfers_group = libadwaita::PreferencesGroup::builder()
        .title("Transfers")
        .build();

    let folder_row = libadwaita::ActionRow::builder()
        .title("Download Folder")
        .subtitle(&config.borrow().download_dir.display().to_string())
        .activatable(true)
        .build();

    let choose_btn = gtk4::Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .tooltip_text("Choose folder")
        .build();
    folder_row.add_suffix(&choose_btn);
    transfers_group.add(&folder_row);
    page.add(&transfers_group);
    prefs.add(&page);

    let config_pick = config.clone();
    let folder_row_pick = folder_row.clone();
    let prefs_weak = prefs.downgrade();
    choose_btn.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::builder()
            .title("Choose Download Folder")
            .modal(true)
            .build();
        let config_c = config_pick.clone();
        let row_c = folder_row_pick.clone();
        let parent = prefs_weak.upgrade().map(|w| w.upcast::<gtk4::Window>());
        dialog.select_folder(
            parent.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        config_c.borrow_mut().download_dir = path.clone();
                        row_c.set_subtitle(&path.display().to_string());
                    }
                }
            },
        );
    });

    prefs.connect_close_request(move |_| {
        let new_name = name_row.text().to_string();
        let mut cfg = config.borrow_mut();

        if !new_name.is_empty() && new_name != cfg.device_name {
            cfg.device_name = new_name.clone();
            window_title.set_subtitle(&new_name);
            let _ = cmd_tx.send_blocking(AppCommand::SetDeviceName(new_name));
        }

        let _ = cmd_tx.send_blocking(AppCommand::SetDownloadDir(cfg.download_dir.clone()));
        let _ = cfg.save();

        glib::Propagation::Proceed
    });

    prefs.present();
}

// ── Peer rows ─────────────────────────────────────────────────────────────────

fn add_peer_row(
    list_box: &gtk4::ListBox,
    id: &str,
    name: &str,
    addr: SocketAddr,
    cmd_tx: &Sender<AppCommand>,
) {
    // Dedup — the ID is the LocalSend fingerprint, stable across re-announcements.
    let mut child = list_box.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == id {
            return;
        }
        child = widget.next_sibling();
    }

    let row = libadwaita::ActionRow::builder()
        .title(name)
        .subtitle(&addr.ip().to_string())
        .activatable(false)
        .build();

    row.set_widget_name(id);

    // Drop target: collect all dropped files into a single SendFiles command
    // so multi-file and folder drops are sent as one transfer (roadmap 3.7).
    let drop = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let cmd_tx = cmd_tx.clone();
    let peer_fp = id.to_string();
    drop.connect_drop(move |_, value, _, _| {
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let paths: Vec<std::path::PathBuf> =
                file_list.files().iter().filter_map(|f| f.path()).collect();
            if !paths.is_empty() {
                let _ = cmd_tx.send_blocking(AppCommand::SendFiles {
                    peer_addr: addr,
                    peer_fingerprint: peer_fp.clone(),
                    paths,
                });
            }
        }
        true
    });
    row.add_controller(drop);
    list_box.append(&row);
}

fn remove_peer_row(list_box: &gtk4::ListBox, id: &str) {
    let mut child = list_box.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Some(row) = widget.downcast_ref::<libadwaita::ActionRow>() {
            if row.widget_name() == id {
                list_box.remove(row);
                return;
            }
        }
        child = next;
    }
}

fn update_stack(list_box: &gtk4::ListBox, stack: &gtk4::Stack) {
    if list_box.first_child().is_some() {
        stack.set_visible_child_name("list");
    } else {
        stack.set_visible_child_name("empty");
    }
}

// ── Transfer request dialog ───────────────────────────────────────────────────

fn show_transfer_request(
    window: &libadwaita::ApplicationWindow,
    transfer_id: String,
    sender_name: String,
    file_name: String,
    file_count: usize,
    size_bytes: u64,
    peer_fingerprint: String,
    cmd_tx: &Sender<AppCommand>,
) {
    let what = if file_count == 1 {
        file_name.clone()
    } else {
        format!("{file_count} files (including {file_name})")
    };

    let dialog = libadwaita::AlertDialog::builder()
        .heading(format!("{sender_name} wants to send you a file"))
        .body(format!(
            "{what}  ({})\n\nVerified identity: {peer_fingerprint}…\n\nDo you want to accept?",
            human_bytes(size_bytes)
        ))
        .default_response("accept")
        .close_response("deny")
        .build();

    dialog.add_response("deny", "Deny");
    dialog.add_response("accept", "Accept");
    dialog.set_response_appearance("accept", libadwaita::ResponseAppearance::Suggested);

    let cmd_tx_accept = cmd_tx.clone();
    let cmd_tx_deny = cmd_tx.clone();
    dialog.connect_response(Some("accept"), {
        let tid = transfer_id.clone();
        move |_, _| {
            let _ = cmd_tx_accept.send_blocking(AppCommand::AcceptTransfer {
                transfer_id: tid.clone(),
            });
        }
    });
    dialog.connect_response(Some("deny"), {
        move |_, _| {
            let _ = cmd_tx_deny.send_blocking(AppCommand::DenyTransfer {
                transfer_id: transfer_id.clone(),
            });
        }
    });

    dialog.present(Some(window));
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn human_bytes(b: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = K * 1024;
    const G: u64 = M * 1024;
    if b >= G {
        format!("{:.1} GB", b as f64 / G as f64)
    } else if b >= M {
        format!("{:.1} MB", b as f64 / M as f64)
    } else if b >= K {
        format!("{:.1} KB", b as f64 / K as f64)
    } else {
        format!("{b} B")
    }
}

/// Format seconds into "1h 23m", "4m 5s", or "42s".
fn format_eta(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}
