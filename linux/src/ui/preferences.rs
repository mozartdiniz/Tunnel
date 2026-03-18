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
    prefs.add(&page);

    let config_pick = config.clone();
    let folder_row_pick = folder_row.clone();
    let prefs_weak = prefs.downgrade();
    choose_btn.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::builder()
            .title("Choose Download Folder")
            .modal(true)
            .build();
        let config_c = config_pick.clone();
        let row_c = folder_row_pick.clone();
        let parent = prefs_weak.upgrade().map(|w| w.upcast::<gtk4::Window>());
        dialog.select_folder(
            parent.as_ref(),
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
    });

    prefs.connect_close_request(move |_| {
        let new_name = name_row.text().to_string();
        let mut cfg = config.borrow_mut();

        if !new_name.is_empty() && new_name != cfg.device_name {
            cfg.device_name = new_name.clone();
            window_title.set_subtitle(&new_name);
            let _ = cmd_tx.send_blocking(AppCommand::SetDeviceName(new_name));
        }

        let _ = cmd_tx.send_blocking(AppCommand::SetDownloadDir(cfg.download_dir.clone()));
        let _ = cfg.save();

        glib::Propagation::Proceed
    });

    prefs.present();
}
