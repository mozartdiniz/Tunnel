use std::path::PathBuf;

use adw::prelude::*;
use glib::subclass::types::{ObjectSubclass, ObjectSubclassIsExt};
use gtk::subclass::widget::WidgetClassExt;
use strum::IntoEnumIterator as _;

use crate::{
    ui::preferences::WarpPreferencesDialog,
    util::{future::main_async_local, show_dir},
};

#[derive(strum::Display, strum::AsRefStr, strum::EnumIter)]
#[strum(prefix = "win.", serialize_all = "kebab-case")]
pub enum Action {
    ShowHelpOverlay,
    Preferences,
    About,
    OpenFile,
    OpenFolder,
    ReceiveFile,
    ShowFile,
}

impl Action {
    pub(super) fn install(
        class: &mut <super::imp::WarpApplicationWindow as ObjectSubclass>::Class,
    ) {
        for action in Self::iter() {
            match action {
                Action::ShowHelpOverlay => {
                    class.add_binding_action(
                        gdk::Key::question,
                        gdk::ModifierType::CONTROL_MASK,
                        action.as_ref(),
                    );
                }
                Action::Preferences => {
                    class.install_action(action.as_ref(), None, move |win, _, _| {
                        WarpPreferencesDialog::new().present(Some(win));
                    });
                    class.add_binding_action(
                        gdk::Key::comma,
                        gdk::ModifierType::CONTROL_MASK,
                        action.as_ref(),
                    );
                }
                Action::About => {
                    class.install_action(action.as_ref(), None, move |win, _, _| {
                        win.show_about_dialog();
                    });
                }
                Action::OpenFile => {
                    class.install_action(action.as_ref(), None, move |win, _, _| {
                        if !win.transfer_in_progress() {
                            win.imp().stack.set_visible_child_name("send");
                            glib::MainContext::default().spawn_local(glib::clone!(
                                #[strong]
                                win,
                                async move {
                                    win.select_file().await;
                                }
                            ));
                        }
                    });
                    class.add_binding_action(
                        gdk::Key::O,
                        gdk::ModifierType::CONTROL_MASK,
                        action.as_ref(),
                    );
                }
                Action::OpenFolder => {
                    class.install_action(action.as_ref(), None, move |win, _, _| {
                        if !win.transfer_in_progress() {
                            win.imp().stack.set_visible_child_name("send");
                            glib::MainContext::default().spawn_local(glib::clone!(
                                #[strong]
                                win,
                                async move {
                                    win.select_folder().await;
                                }
                            ));
                        }
                    });
                    class.add_binding_action(
                        gdk::Key::D,
                        gdk::ModifierType::CONTROL_MASK,
                        action.as_ref(),
                    );
                }
                Action::ReceiveFile => {
                    class.install_action(action.as_ref(), None, move |win, _, _| {
                        if !win.transfer_in_progress() {
                            win.imp().stack.set_visible_child_name("receive");
                            win.imp().code_entry.grab_focus();
                        }
                    });
                    class.add_binding_action(
                        gdk::Key::R,
                        gdk::ModifierType::CONTROL_MASK,
                        action.as_ref(),
                    );
                }
                Action::ShowFile => {
                    class.install_action(
                        action.as_ref(),
                        Some(&PathBuf::static_variant_type()),
                        move |_win, _, data| {
                            if let Some(data) = data {
                                let path = PathBuf::from_variant(data);
                                if let Some(filename) = path {
                                    main_async_local(
                                        crate::util::error::AppError::handle,
                                        async move { show_dir(&filename).await },
                                    );
                                }
                            }
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::Action;

    #[test]
    fn strum_serialisations() {
        assert_eq!(Action::ShowHelpOverlay.as_ref(), "win.show-help-overlay");
        assert_eq!(Action::ShowFile.as_ref(), "win.show-file");
    }
}
