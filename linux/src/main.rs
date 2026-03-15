mod app;
mod config;
mod discovery;
mod protocol;
mod tls;
mod transfer;
mod ui;

use anyhow::Result;
use gtk4::prelude::*;

fn main() -> Result<()> {
    // Install ring as the rustls crypto provider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tunnel=debug".parse().unwrap()),
        )
        .init();

    let config = config::Config::load().unwrap_or_default();

    // Channels: tokio tasks -> GTK (events)
    let (event_tx, event_rx) = async_channel::unbounded::<app::AppEvent>();
    // Channels: GTK -> tokio tasks (commands)
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<app::AppCommand>();

    // Run the network stack in a dedicated OS thread with its own tokio runtime.
    // GTK must own the main thread.
    let config_clone = config.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async move {
            if let Err(e) = app::run_network(config_clone, event_tx, cmd_rx).await {
                tracing::error!("Network layer crashed: {e:#}");
            }
        });
    });

    // GTK application runs on the main thread
    let gtk_app = libadwaita::Application::builder()
        .application_id("dev.tunnel.Tunnel")
        .build();

    gtk_app.connect_activate(move |app| {
        ui::build_ui(app, config.clone(), event_rx.clone(), cmd_tx.clone());
    });

    gtk_app.run();

    Ok(())
}
