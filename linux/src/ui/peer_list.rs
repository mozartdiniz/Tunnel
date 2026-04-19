use std::net::SocketAddr;

use async_channel::Sender;
use gtk4::gdk;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::app::AppCommand;
use crate::ui::{format_eta, human_bytes, TransferState};

pub fn add_peer_row(
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
        .subtitle("Tap or drop a file to send")
        .activatable(true)
        .build();

    row.set_widget_name(id);

    // Click: open file chooser dialog.
    let cmd_tx_click = cmd_tx.clone();
    let peer_fp_click = id.to_string();
    row.connect_activated(move |row| {
        let dialog = gtk4::FileDialog::builder()
            .title("Select files to send")
            .modal(true)
            .build();
        let window = row.root().and_downcast::<gtk4::Window>();
        let cmd_tx3 = cmd_tx_click.clone();
        let peer_fp3 = peer_fp_click.clone();
        dialog.open_multiple(window.as_ref(), gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(files) = result {
                let paths: Vec<std::path::PathBuf> = (0..files.n_items())
                    .filter_map(|i| {
                        files
                            .item(i)
                            .and_downcast::<gtk4::gio::File>()
                            .and_then(|f| f.path())
                    })
                    .collect();
                if !paths.is_empty() {
                    let _ = cmd_tx3.try_send(AppCommand::SendFiles {
                        peer_addr: addr,
                        peer_fingerprint: peer_fp3.clone(),
                        paths,
                    });
                }
            }
        });
    });

    // Drop target: collect all dropped files into a single SendFiles command.
    let drop = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let cmd_tx = cmd_tx.clone();
    let peer_fp = id.to_string();
    drop.connect_drop(move |_, value, _, _| {
        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let paths: Vec<std::path::PathBuf> =
                file_list.files().iter().filter_map(|f| f.path()).collect();
            if !paths.is_empty() {
                let _ = cmd_tx.try_send(AppCommand::SendFiles {
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

/// Update the subtitle of a peer row to reflect its current transfer state.
pub fn update_peer_row_progress(
    list_box: &gtk4::ListBox,
    peer_id: &str,
    state: &TransferState,
) {
    let mut child = list_box.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == peer_id {
            if let Some(row) = widget.downcast_ref::<libadwaita::ActionRow>() {
                let subtitle = transfer_subtitle(state);
                row.set_subtitle(&subtitle);
            }
            return;
        }
        child = widget.next_sibling();
    }
}

fn transfer_subtitle(state: &TransferState) -> String {
    match state {
        TransferState::Idle => "Tap or drop a file to send".to_string(),
        TransferState::Transferring { bytes_done, total_bytes, bytes_per_sec, eta_secs } => {
            let pct = if *total_bytes > 0 {
                (*bytes_done as f64 / *total_bytes as f64 * 100.0) as u64
            } else {
                0
            };
            let speed = if *bytes_per_sec > 0 {
                format!("  {}ps", human_bytes(*bytes_per_sec))
            } else {
                String::new()
            };
            let eta = match eta_secs {
                Some(s) => format!("  ETA {}", format_eta(*s)),
                None => String::new(),
            };
            format!(
                "{}% · {} / {}{}{}",
                pct,
                human_bytes(*bytes_done),
                human_bytes(*total_bytes),
                speed,
                eta
            )
        }
        TransferState::Syncing => "Syncing…".to_string(),
        TransferState::Complete => "Transfer complete ✓".to_string(),
        TransferState::Error(_) => "Drop a file to send".to_string(),
    }
}

pub fn remove_peer_row(list_box: &gtk4::ListBox, id: &str) {
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

pub fn update_stack(list_box: &gtk4::ListBox, stack: &gtk4::Stack) {
    if list_box.first_child().is_some() {
        stack.set_visible_child_name("list");
    } else {
        stack.set_visible_child_name("empty");
    }
}
