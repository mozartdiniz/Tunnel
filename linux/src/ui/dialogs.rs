use async_channel::Sender;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::app::AppCommand;
use super::helpers::human_bytes;

pub fn show_transfer_request(
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
            let _ = cmd_tx_accept.try_send(AppCommand::AcceptTransfer {
                transfer_id: tid.clone(),
            });
        }
    });
    dialog.connect_response(Some("deny"), {
        move |_, _| {
            let _ = cmd_tx_deny.try_send(AppCommand::DenyTransfer {
                transfer_id: transfer_id.clone(),
            });
        }
    });

    dialog.present(Some(window));
}
