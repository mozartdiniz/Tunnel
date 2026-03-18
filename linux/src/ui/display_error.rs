use std::fmt::Display;

use libadwaita::prelude::*;

/// Extension trait that surfaces a `Result`'s error as an Adwaita toast.
///
/// Modelled after Shortwave's `DisplayError` pattern: call `.handle_error("…")`
/// on any `Result` to log the error and show a dismissible toast to the user.
pub trait DisplayError {
    /// Show the error (if any) as a toast on `overlay`, and log it via `tracing`.
    fn handle_error(self, overlay: &libadwaita::ToastOverlay, title: &str);
}

impl<T, E: Display> DisplayError for Result<T, E> {
    fn handle_error(self, overlay: &libadwaita::ToastOverlay, title: &str) {
        if let Err(e) = self {
            tracing::warn!("{title}: {e}");
            let toast = libadwaita::Toast::builder()
                .title(title)
                .timeout(5)
                .build();
            overlay.add_toast(toast);
        }
    }
}

/// Show a plain error message as a toast without a `Result`.
pub fn show_error(overlay: &libadwaita::ToastOverlay, title: &str) {
    let toast = libadwaita::Toast::builder()
        .title(title)
        .timeout(5)
        .build();
    overlay.add_toast(toast);
}
