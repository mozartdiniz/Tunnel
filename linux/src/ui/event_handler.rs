use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;

use async_channel::Sender;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::app::{AppCommand, AppEvent};

use super::dialogs::show_transfer_request;
use super::helpers::{format_eta, human_bytes, set_status};
use super::notifications::{send_complete_notification, send_incoming_notification};
use super::peer_list::{add_peer_row, remove_peer_row, update_stack};

pub fn handle_event(
    event: AppEvent,
    list_box: &gtk4::ListBox,
    stack: &gtk4::Stack,
    status_dot: &gtk4::Label,
    progress_bar: &gtk4::ProgressBar,
    peers: &RefCell<HashMap<String, (String, SocketAddr)>>,
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
