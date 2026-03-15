/// GTK4 / Libadwaita user interface.
///
/// Layout:
///   AdwApplicationWindow
///   └── AdwToolbarView
///       ├── AdwHeaderBar  (top)
///       ├── Main content
///       │   ├── "No devices" placeholder  (shown when list is empty)
///       │   └── ScrolledWindow > ListBox  (device rows)
///       └── Status bar label             (bottom)
///
/// Drag-and-drop: the window accepts file drops.
/// When a file is dropped, the UI checks which device row the cursor is over
/// and sends `AppCommand::SendFile` through the command channel.
use std::collections::HashMap;
use std::net::SocketAddr;

use async_channel::{Receiver, Sender};
use gtk4::gdk;
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
    // ── If first run (no device name saved), show setup dialog first ─────────
    // For now we use the default config; a setup dialog can be added later.
    // TODO: show AdwDialog asking for device name if config was never saved.

    let window = build_main_window(app, config.clone(), event_rx, cmd_tx);
    window.present();
}

fn build_main_window(
    app: &libadwaita::Application,
    config: Config,
    event_rx: Receiver<AppEvent>,
    cmd_tx: Sender<AppCommand>,
) -> libadwaita::ApplicationWindow {
    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Tunnel")
        .default_width(420)
        .default_height(560)
        .build();

    // ── Layout ────────────────────────────────────────────────────────────────
    let toolbar_view = libadwaita::ToolbarView::new();
    window.set_content(Some(&toolbar_view));

    // Header bar
    let header = libadwaita::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    // Device name label in the header
    let device_label = gtk4::Label::builder()
        .label(&format!("📡  {}", config.device_name))
        .css_classes(["caption"])
        .build();
    header.pack_start(&device_label);

    // Main content box
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(0)
        .build();
    toolbar_view.set_content(Some(&content));

    // ── Device list ───────────────────────────────────────────────────────────
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
    content.append(&scrolled);

    // Empty state placeholder
    let empty_label = gtk4::Label::builder()
        .label("Searching for devices on your network…")
        .css_classes(["dim-label"])
        .vexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    content.append(&empty_label);

    // Status bar at the bottom
    let status_bar = gtk4::Label::builder()
        .label("Ready")
        .css_classes(["caption", "dim-label"])
        .margin_bottom(8)
        .build();
    toolbar_view.add_bottom_bar(&status_bar);

    // ── Drag-and-drop ─────────────────────────────────────────────────────────
    // peer_addr is tracked via cursor position hack for simplicity.
    // Each device row will install its own DropTarget.
    // (Wired up in `add_peer_row` below.)

    // ── Track peers in a shared map (id → SocketAddr) ────────────────────────
    // We use a Rc<RefCell<...>> because GTK closures are single-threaded.
    let peers: std::rc::Rc<std::cell::RefCell<HashMap<String, (String, SocketAddr)>>> =
        std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()));

    // ── Event loop: receive AppEvents from the network thread ─────────────────
    let list_box_clone = list_box.clone();
    let empty_label_clone = empty_label.clone();
    let status_bar_clone = status_bar.clone();
    let peers_clone = peers.clone();
    let cmd_tx_clone = cmd_tx.clone();
    let window_clone = window.clone();

    glib::MainContext::default().spawn_local(async move {
        while let Ok(event) = event_rx.recv().await {
            handle_event(
                event,
                &list_box_clone,
                &empty_label_clone,
                &status_bar_clone,
                &peers_clone,
                &cmd_tx_clone,
                &window_clone,
            );
        }
    });

    window
}

fn handle_event(
    event: AppEvent,
    list_box: &gtk4::ListBox,
    empty_label: &gtk4::Label,
    status_bar: &gtk4::Label,
    peers: &std::rc::Rc<std::cell::RefCell<HashMap<String, (String, SocketAddr)>>>,
    cmd_tx: &Sender<AppCommand>,
    window: &libadwaita::ApplicationWindow,
) {
    match event {
        AppEvent::PeerFound { id, name, addr } => {
            peers.borrow_mut().insert(id.clone(), (name.clone(), addr));
            add_peer_row(list_box, &id, &name, addr, cmd_tx);
            update_empty_state(list_box, empty_label);
        }

        AppEvent::PeerLost { id } => {
            peers.borrow_mut().remove(&id);
            remove_peer_row(list_box, &id);
            update_empty_state(list_box, empty_label);
            status_bar.set_label(&format!("Device '{id}' left the network"));
        }

        AppEvent::IncomingRequest {
            transfer_id,
            sender_name,
            file_name,
            size_bytes,
        } => {
            show_transfer_request(
                window,
                transfer_id,
                sender_name,
                file_name,
                size_bytes,
                cmd_tx,
            );
        }

        AppEvent::TransferProgress {
            bytes_done,
            total_bytes,
            ..
        } => {
            let pct = bytes_done as f64 / total_bytes as f64 * 100.0;
            status_bar.set_label(&format!(
                "Transferring… {:.1}%  ({} / {})",
                pct,
                human_bytes(bytes_done),
                human_bytes(total_bytes)
            ));
        }

        AppEvent::TransferComplete { saved_to, .. } => {
            status_bar.set_label(&format!(
                "✓  Saved to {}",
                saved_to.display()
            ));
        }

        AppEvent::TransferError { message, .. } => {
            status_bar.set_label(&format!("✗  {message}"));
        }
    }
}

/// Add a device row with its own drop target.
fn add_peer_row(
    list_box: &gtk4::ListBox,
    id: &str,
    name: &str,
    addr: SocketAddr,
    cmd_tx: &Sender<AppCommand>,
) {
    let row = libadwaita::ActionRow::builder()
        .title(name)
        .subtitle(&addr.to_string())
        .activatable(false)
        .build();

    // Tag the row with the peer id so we can remove it later
    row.set_widget_name(id);

    // Drop target: accepts file lists
    let drop = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let cmd_tx = cmd_tx.clone();
    drop.connect_drop(move |_, value, _, _| {
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            for file in file_list.files() {
                if let Some(path) = file.path() {
                    let _ = cmd_tx.send_blocking(AppCommand::SendFile {
                        peer_addr: addr,
                        file_path: path,
                    });
                }
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

fn update_empty_state(list_box: &gtk4::ListBox, empty_label: &gtk4::Label) {
    let has_items = list_box.first_child().is_some();
    list_box.set_visible(has_items);
    empty_label.set_visible(!has_items);
}

/// Show a confirmation dialog for an incoming transfer request.
fn show_transfer_request(
    window: &libadwaita::ApplicationWindow,
    transfer_id: String,
    sender_name: String,
    file_name: String,
    size_bytes: u64,
    cmd_tx: &Sender<AppCommand>,
) {
    let dialog = libadwaita::AlertDialog::builder()
        .heading(format!("{sender_name} wants to send you a file"))
        .body(format!(
            "{file_name}  ({})\n\nDo you want to accept?",
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
