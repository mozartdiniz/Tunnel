use gtk4::gio;
use gtk4::prelude::*;

use super::helpers::human_bytes;

pub fn send_incoming_notification(
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

pub fn send_complete_notification(saved_to: &std::path::PathBuf) {
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
