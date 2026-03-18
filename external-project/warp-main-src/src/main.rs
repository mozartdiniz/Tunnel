#![warn(
    clippy::get_unwrap,
    clippy::if_then_some_else_none,
    clippy::let_underscore_must_use,
    clippy::mem_forget,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::rc_buffer,
    clippy::rc_mutex,
    clippy::separated_literal_suffix,
    clippy::string_to_string,
    clippy::str_to_string,
    clippy::todo,
    clippy::try_err,
    clippy::undocumented_unsafe_blocks,
    clippy::unneeded_field_pattern
)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::unused_self,
    clippy::wildcard_imports,
    clippy::new_without_default
)]
// Hide the console on Windows
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod config;
#[allow(dead_code)]
mod gettext;
#[allow(dead_code)]
mod globals;
mod ui;
mod util;

use std::path::PathBuf;

use crate::ui::application::WarpApplication;
use crate::util::error;
use gettextrs::gettext;

fn setup_gresources() {
    #[cfg(all(debug_assertions, not(feature = "meson")))]
    let data = {
        log::info!("Loading GResource data from directory 'data/resources'");
        gvdb::gresource::BundleBuilder::from_directory(
            "/app/drey/Warp",
            &std::path::PathBuf::from("data/resources"),
            true,
            true,
        )
        .map_or_else(
            |_| {
                log::warn!(
                "Loading fallback precompiled GResource data because the directory does not exist"
            );
                glib::Bytes::from(globals::GRESOURCE_DATA)
            },
            |builder| glib::Bytes::from(&builder.build().unwrap()),
        )
    };
    #[cfg(any(not(debug_assertions), feature = "meson"))]
    let data = {
        log::debug!("Loading precompiled GResource data");
        glib::Bytes::from(globals::GRESOURCE_DATA)
    };

    let resource = gio::Resource::from_data(&data).expect("Error loading resource bundle");
    gio::resources_register(&resource);
}

#[cfg(target_os = "windows")]
/// Workarounds and hacks for Windows
fn windows_hacks() {
    // stdout support with hidden console
    let _ = win32console::console::WinConsole::free_console();
    // ATTACH_PARENT_PROCESS = 0xFFFFFFFF
    let _ = win32console::console::WinConsole::attach_console(0xFFFFFFFF);
}

fn main() {
    // Initialize logger
    pretty_env_logger::init();

    error::install_panic_hook();

    #[cfg(target_os = "windows")]
    windows_hacks();

    // Prepare i18n
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");

    let localedir: PathBuf = {
        #[cfg(target_os = "linux")]
        {
            option_env!("LOCALEDIR")
                .unwrap_or(globals::DEFAULT_LOCALEDIR_LINUX)
                .into()
        }
        #[cfg(target_os = "windows")]
        {
            let mut base_dir = globals::WINDOWS_BASE_PATH.clone();
            base_dir.push("share\\locale");
            base_dir
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            compile_error!("Missing platform support");
        }
    };

    match gettextrs::bindtextdomain(globals::GETTEXT_PACKAGE, localedir) {
        Ok(path) => log::debug!("Bound text domain for path: {}", path.display()),
        Err(err) => log::error!("Error binding text domain: {}", err),
    }
    if let Err(err) = gettextrs::bind_textdomain_codeset(globals::GETTEXT_PACKAGE, "UTF-8") {
        log::error!(
            "Error while setting locale formatting: {}. This shouldn't be a problem—except on Windows",
            err
        );
    }

    if let Err(err) = gettextrs::textdomain(globals::GETTEXT_PACKAGE) {
        log::error!("Unable to switch to the text domain: {}", err);
    }

    glib::set_application_name(&gettext("Warp"));

    setup_gresources();

    let app = WarpApplication::new();
    app.run();
}
