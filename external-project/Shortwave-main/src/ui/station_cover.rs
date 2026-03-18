// Shortwave - station_cover.rs
// Copyright (C) 2024  Felix Häcker <haeckerfelix@gnome.org>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::cell::Cell;
use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash, Hasher};

use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::{Properties, clone, subclass};
use gtk::{CompositeTemplate, gdk, gio, glib, pango};

use crate::api::SwStation;
use crate::app::SwApplication;
use crate::ui::SwScalableImage;

mod imp {
    use super::*;
    static MAX_COVER_SIZE: i32 = 256;

    #[derive(Debug, Default, Properties, CompositeTemplate)]
    #[template(resource = "/de/haeckerfelix/Shortwave/gtk/station_cover.ui")]
    #[properties(wrapper_type = super::SwStationCover)]
    pub struct SwStationCover {
        #[template_child]
        image: TemplateChild<SwScalableImage>,
        #[template_child]
        stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        fallback_label: TemplateChild<gtk::Label>,

        #[property(get, set=Self::set_size)]
        size: Cell<i32>,
        #[property(get, set=Self::set_station, nullable)]
        station: RefCell<Option<SwStation>>,
        #[property(get)]
        is_loaded: Cell<bool>,

        loader_cancellable: RefCell<Option<gio::Cancellable>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SwStationCover {
        const NAME: &'static str = "SwStationCover";
        type ParentType = adw::Bin;
        type Type = super::SwStationCover;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("cover");
            Self::bind_template(klass);
        }

        fn instance_init(obj: &subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SwStationCover {}

    impl WidgetImpl for SwStationCover {
        fn map(&self) {
            self.parent_map();

            self.update_initials();
            self.update_font_size();
            self.update_cover();
        }

        fn unmap(&self) {
            self.parent_unmap();
            self.cancel();
        }
    }

    impl BinImpl for SwStationCover {}

    impl SwStationCover {
        fn set_size(&self, size: i32) {
            self.size.set(size);

            self.image.set_size_request(size, size);
            self.obj().set_size_request(size, size);

            self.update_font_size();
        }

        fn set_station(&self, station: Option<&SwStation>) {
            *self.station.borrow_mut() = station.cloned();

            // Reset previous cover
            self.image.set_texture(gdk::Texture::NONE);
            self.stack.set_visible_child_name("fallback");

            self.is_loaded.set(false);
            self.obj().notify_is_loaded();

            // Set fallback initials
            self.update_initials();
            self.update_font_size();

            // Load new cover, but only if it's mapped
            if self.obj().is_mapped() {
                self.update_cover();
            }

            if let Some(station) = self.obj().station() {
                station.connect_metadata_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.is_loaded.set(false);
                        imp.update_cover();
                        imp.update_initials();
                    }
                ));

                station.connect_custom_cover_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.is_loaded.set(false);
                        imp.update_cover();
                    }
                ));
            }
        }

        fn update_initials(&self) {
            if !self.obj().is_mapped() {
                return;
            }

            let title = if let Some(station) = self.obj().station() {
                station.title()
            } else {
                String::new()
            };

            let mut initials = String::new();
            let words: Vec<&str> = title.split(" ").collect();

            if let Some(char) = words.first().and_then(|w| Self::first_char(w)) {
                initials += &char.to_string();
            }

            if let Some(char) = words.get(1).and_then(|w| Self::first_char(w)) {
                initials += &char.to_string();
            }

            self.fallback_label.set_label(&initials.to_uppercase());
            self.update_color_class();
        }

        fn first_char(word: &str) -> Option<char> {
            word.chars()
                .filter(|c| c.is_alphabetic())
                .collect::<Vec<char>>()
                .first()
                .cloned()
        }

        fn update_font_size(&self) {
            if !self.obj().is_mapped() {
                return;
            }

            let absolute_size = match self.obj().size() {
                64 => 29491,
                128 => 58982,
                192 => 88473,
                256 => 117964,
                _ => 0,
            };

            let attributes = pango::AttrList::new();
            attributes.insert(pango::AttrSize::new_size_absolute(absolute_size));
            self.fallback_label.set_attributes(Some(&attributes));
        }

        fn update_color_class(&self) {
            for css_class in self.fallback_label.css_classes() {
                self.fallback_label.remove_css_class(&css_class);
            }

            if let Some(station) = self.obj().station() {
                let mut hasher = DefaultHasher::new();
                let s = if !station.title().is_empty() {
                    station.title()
                } else {
                    "a".to_string()
                };
                s.hash(&mut hasher);
                let hash = hasher.finish();

                let color_class = hash % 14;
                self.fallback_label
                    .add_css_class(&format!("color{color_class}"));
            }
        }

        fn update_cover(&self) {
            glib::spawn_future_local(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load_cover().await;
                }
            ));
        }

        async fn load_cover(&self) {
            if self.obj().is_loaded() || !self.obj().is_mapped() {
                return;
            }

            self.cancel();

            if let Some(station) = self.obj().station() {
                // First check whether we have some custom cover for that station
                // Usually only for local added stations
                if let Some(texture) = station.custom_cover() {
                    self.image.set_texture(Some(&texture));
                    self.stack.set_visible_child_name("image");

                    self.is_loaded.set(true);
                    self.obj().notify_is_loaded();
                } else if let Some(favicon_url) = station.metadata().favicon {
                    let mut cover_loader = SwApplication::default().cover_loader();

                    let cancellable = gio::Cancellable::new();
                    *self.loader_cancellable.borrow_mut() = Some(cancellable.clone());

                    let size = MAX_COVER_SIZE * self.obj().scale_factor();
                    let res = cover_loader
                        .load_cover(&favicon_url, size, cancellable.clone())
                        .await;

                    match res {
                        Ok(texture) => {
                            self.image.set_texture(Some(&texture));
                            self.stack.set_visible_child_name("image");

                            self.is_loaded.set(true);
                            self.obj().notify_is_loaded();
                        }
                        Err(e) => {
                            if e.root_cause().to_string() != "cancelled" {
                                warn!(
                                    "Unable to load cover for station {:?} ({:?}): {}",
                                    station.title(),
                                    station.metadata().favicon.map(|f| f.to_string()),
                                    e.root_cause()
                                )
                            }
                        }
                    }
                } else {
                    self.stack.set_visible_child_name("fallback");
                }
            } else {
                self.stack.set_visible_child_name("fallback");
            }
        }

        fn cancel(&self) {
            if let Some(cancellable) = self.loader_cancellable.borrow_mut().take() {
                cancellable.cancel();
            }
        }
    }
}

glib::wrapper! {
    pub struct SwStationCover(ObjectSubclass<imp::SwStationCover>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SwStationCover {
    fn default() -> Self {
        glib::Object::new()
    }
}

mod imp_animated {
    use super::*;

    #[derive(Debug, Default, Properties)]
    #[properties(wrapper_type = super::SwStationCoverAnimated)]
    pub struct SwStationCoverAnimated {
        cover1: SwStationCover,
        cover2: SwStationCover,
        stack: adw::ViewStack,

        #[property(get, set)]
        size: Cell<i32>,
        #[property(get, set=Self::set_station, nullable)]
        station: RefCell<Option<SwStation>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SwStationCoverAnimated {
        const NAME: &'static str = "SwStationCoverAnimated";
        type ParentType = adw::Bin;
        type Type = super::SwStationCoverAnimated;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SwStationCoverAnimated {
        fn constructed(&self) {
            self.parent_constructed();

            self.obj().set_child(Some(&self.stack));
            self.stack.add_named(&self.cover1, Some("cover1"));
            self.stack.add_named(&self.cover2, Some("cover2"));
            self.stack.set_enable_transitions(true);

            self.obj().set_halign(gtk::Align::Center);
            self.obj().set_valign(gtk::Align::Center);

            self.obj()
                .bind_property("size", &self.cover1, "size")
                .build();
            self.obj()
                .bind_property("size", &self.cover2, "size")
                .build();
        }
    }

    impl WidgetImpl for SwStationCoverAnimated {}

    impl BinImpl for SwStationCoverAnimated {}

    impl SwStationCoverAnimated {
        fn set_station(&self, station: Option<&SwStation>) {
            *self.station.borrow_mut() = station.cloned();

            let new_cover = if self.stack.visible_child_name().unwrap() == "cover1" {
                self.cover2.set_station(station);
                &self.cover2
            } else {
                self.cover1.set_station(station);
                &self.cover1
            };

            self.stack.set_visible_child(new_cover);
        }
    }
}

glib::wrapper! {
    /// Transitions between station covers. Only useful when the underlying [SwStation] can change, for example in player widgets.
    pub struct SwStationCoverAnimated(ObjectSubclass<imp_animated::SwStationCoverAnimated>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
