use std::cell::RefCell;
use std::rc::Rc;

use async_channel::Sender;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::app::AppCommand;
use crate::config::Config;

pub fn show_preferences(
    parent: &libadwaita::ApplicationWindow,
    config: Rc<RefCell<Config>>,
    cmd_tx: Sender<AppCommand>,
    window_title: libadwaita::WindowTitle,
) {
    let prefs = libadwaita::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .title("Settings")
        .build();

    let page = libadwaita::PreferencesPage::new();

    let device_group = libadwaita::PreferencesGroup::builder()
        .title("Device")
        .build();

    let name_row = libadwaita::EntryRow::builder()
        .title("Device Name")
        .text(&config.borrow().device_name)
        .build();
    device_group.add(&name_row);
    page.add(&device_group);

    let transfers_group = libadwaita::PreferencesGroup::builder()
        .title("Transfers")
        .build();

    let folder_row = libadwaita::ActionRow::builder()
        .title("Download Folder")
        .subtitle(&config.borrow().download_dir.display().to_string())
        .activatable(true)
        .build();

    let choose_btn = gtk4::Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .tooltip_text("Choose folder")
        .build();
    folder_row.add_suffix(&choose_btn);
    transfers_group.add(&folder_row);
    page.add(&transfers_group);

    // ── Sync group ────────────────────────────────────────────────────────────
    let sync_group = libadwaita::PreferencesGroup::builder()
        .title("Sync")
        .description("When both devices have a sync folder set, files are kept in sync automatically.")
        .build();

    let sync_subtitle = config
        .borrow()
        .sync_folder
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "Not configured".to_string());

    let sync_row = libadwaita::ActionRow::builder()
        .title("Sync Folder")
        .subtitle(&sync_subtitle)
        .activatable(true)
        .build();

    let sync_choose_btn = gtk4::Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .tooltip_text("Choose sync folder")
        .build();
    let sync_clear_btn = gtk4::Button::builder()
        .icon_name("edit-clear-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .tooltip_text("Clear sync folder")
        .build();
    sync_row.add_suffix(&sync_clear_btn);
    sync_row.add_suffix(&sync_choose_btn);
    sync_group.add(&sync_row);
    page.add(&sync_group);

    prefs.add(&page);

    let config_pick = config.clone();
    choose_btn.connect_clicked(glib::clone!(#[weak] folder_row, #[weak] prefs, move |_| {
        let dialog = gtk4::FileDialog::builder()
            .title("Choose Download Folder")
            .modal(true)
            .build();
        let config_c = config_pick.clone();
        let row_c = folder_row.clone();
        dialog.select_folder(
            Some(&prefs).map(|w| w.upcast_ref::<gtk4::Window>()),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        config_c.borrow_mut().download_dir = path.clone();
                        row_c.set_subtitle(&path.display().to_string());
                    }
                }
            },
        );
    }));

    let config_sync = config.clone();
    sync_choose_btn.connect_clicked(glib::clone!(#[weak] sync_row, #[weak] prefs, move |_| {
        let dialog = gtk4::FileDialog::builder()
            .title("Choose Sync Folder")
            .modal(true)
            .build();
        let config_c = config_sync.clone();
        let row_c = sync_row.clone();
        dialog.select_folder(
            Some(&prefs).map(|w| w.upcast_ref::<gtk4::Window>()),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        config_c.borrow_mut().sync_folder = Some(path.clone());
                        row_c.set_subtitle(&path.display().to_string());
                    }
                }
            },
        );
    }));

    let config_clear = config.clone();
    sync_clear_btn.connect_clicked(glib::clone!(#[weak] sync_row, move |_| {
        config_clear.borrow_mut().sync_folder = None;
        sync_row.set_subtitle("Not configured");
    }));

    prefs.connect_close_request(glib::clone!(
        #[weak] name_row,
        #[upgrade_or] glib::Propagation::Proceed,
        move |_| {
            let new_name = name_row.text().to_string();
            let mut cfg = config.borrow_mut();

            if !new_name.is_empty() && new_name != cfg.device_name {
                cfg.device_name = new_name.clone();
                window_title.set_subtitle(&new_name);
                let _ = cmd_tx.try_send(AppCommand::SetDeviceName(new_name));
            }

            let _ = cmd_tx.try_send(AppCommand::SetDownloadDir(cfg.download_dir.clone()));
            let _ = cmd_tx.try_send(AppCommand::SetSyncFolder(cfg.sync_folder.clone()));
            let _ = cfg.save();

            glib::Propagation::Proceed
        }
    ));

    prefs.present();
}
