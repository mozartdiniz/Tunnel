mod app;
mod application;
mod config;
mod discovery;
mod inhibit;
mod localsend;
mod tls;
mod transfer;
mod ui;
mod window;

use gtk4::prelude::*;

fn main() -> gtk4::glib::ExitCode {
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
