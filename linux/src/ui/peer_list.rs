use std::net::SocketAddr;

use async_channel::Sender;
use gtk4::gdk;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::app::AppCommand;

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
