// On Windows, suppress the console window for a GUI-only experience.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod application;
mod config;
mod discovery;
mod inhibit;
mod localsend;
mod sync;
mod tls;
mod transfer;
mod ui;
mod window;

use gtk4::prelude::*;

/// On Windows, `windows_subsystem = "windows"` hides the console window but
/// also detaches stdout/stderr. Re-attach to the parent console (if any) so
/// that running from a terminal still shows log output.
#[cfg(target_os = "windows")]
fn windows_hacks() {
    let _ = win32console::console::WinConsole::free_console();
    // ATTACH_PARENT_PROCESS = 0xFFFFFFFF
    let _ = win32console::console::WinConsole::attach_console(0xFFFFFFFF);
}

fn main() -> gtk4::glib::ExitCode {
    #[cfg(target_os = "windows")]
    windows_hacks();
    // Embed and register the compiled GResource bundle (CSS, UI templates, icons).
    gtk4::gio::resources_register_include!("tunnel.gresource")
        .expect("Failed to register GResources");

    // Install ring as the rustls crypto provider.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Structured logging (respects RUST_LOG env var; defaults to debug for this crate).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tunnel=debug".parse().unwrap()),
        )
        .init();

    application::Application::new().run()
}
