use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    hash::Hash,
    rc::Rc,
    sync::OnceLock,
};

use futures::channel::oneshot;
use glib::GString;
use serde::Deserialize;

use crate::gettext::*;

#[derive(Clone, Debug, Deserialize)]
struct Crate {
    name: String,
    version: String,
    authors: Vec<String>,
    license: String,
}

#[derive(Clone, Debug, Deserialize)]
struct UsedBy {
    #[serde(rename = "crate")]
    c: Crate,
}

#[derive(Clone, Debug, Deserialize)]
struct License {
    id: String,
    name: String,
    text: String,
    used_by: Vec<UsedBy>,
}

#[derive(Clone, Debug, Deserialize)]
struct Licenses {
    #[serde(rename = "licenses")]
    licenses: Vec<License>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LegalSection {
    pub title: GString,
    pub copyright: Option<String>,
    pub license_type: gtk::License,
    pub license: Option<String>,
}

#[derive(Debug)]
struct AboutLicense {
    spdx: String,
    name: String,
    license: Option<&'static dyn license::License>,
    escaped_text: Option<String>,
}

impl PartialEq for AboutLicense {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.id() == other.id()
            && self.escaped_text == other.escaped_text
    }
}

impl Eq for AboutLicense {}

impl PartialOrd for AboutLicense {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for AboutLicense {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.spdx.hash(state);
        self.name.hash(state);
        self.id().hash(state);
        self.escaped_text.hash(state);
    }
}

impl Ord for AboutLicense {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.spdx.cmp(&other.spdx) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        match self.name.cmp(&other.name) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        match self.id().cmp(other.id()) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        self.escaped_text.cmp(&other.escaped_text)
    }
}

impl AboutLicense {
    pub fn from_spdx(spdx: &str, name: &str, text: &str) -> Self {
        let license: Option<&dyn license::License> = spdx.parse().ok();
        let text = Self::parse_extra_text(license, text);

        Self {
            spdx: spdx.to_owned(),
            name: name.to_owned(),
            license,
            escaped_text: text,
        }
    }

    pub fn id(&self) -> &str {
        if let Some(license) = &self.license {
            license.id()
        } else {
            &self.spdx
        }
    }

    fn parse_extra_text(license: Option<&dyn license::License>, text: &str) -> Option<String> {
        let need_text = !matches!(
            license.map(license::License::id),
            Some(
                "Apache-2.0"
                    | "GPL-3.0"
                    | "GPL-3.0-or-later"
                    | "EUPL-1.0"
                    | "EUPL-1.1"
                    | "EUPL-1.2"
                    | "MPL-2.0"
                    | "Unicode-DFS-2016"
                    | "Unicode-3.0",
            ),
        );

        need_text.then(|| glib::markup_escape_text(text.trim()).to_string())
    }
}

impl std::fmt::Display for AboutLicense {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parsed: Result<&dyn license::License, _> = self.spdx.parse();
        let info = if let Ok(license) = parsed {
            let url = format!("https://spdx.org/licenses/{}.html", license.id());

            // Translators: License information with a link to the software license
            gettextf(
                "Licensed under the <a href=\"{}\">{}</a>.",
                &[&url, &license.name()],
            )
        } else {
            // Translators: License information without a link to the software license
            gettextf("Licensed under the {}.", &[&self.name])
        };

        if let Some(text) = &self.escaped_text {
            write!(f, "{info}\n\n{text}")
        } else {
            write!(f, "{info}")
        }
    }
}

fn license_info() -> Vec<License> {
    let paths = glib::system_data_dirs()
        .into_iter()
        .map(|p| p.join(crate::globals::APP_NAME).join("licenses.json"))
        .collect::<Vec<_>>();

    for path in &paths {
        if let Ok(file) = std::fs::File::open(path) {
            log::debug!("Loading licenses.json from {}", path.display());
            let licenses: std::result::Result<Licenses, _> = serde_json::from_reader(file);
            return match licenses {
                Ok(licenses) => licenses.licenses,
                Err(err) => {
                    log::warn!("Error loading licenses.json: {}", err);
                    Vec::new()
                }
            };
        }
    }

    log::warn!(
        "Error loading licenses.json: File not found in system data directories: {:?}",
        &paths
    );

    Vec::new()
}

#[derive(Debug, Hash, PartialEq, Eq)]
struct SectionKey {
    authors: Vec<String>,
    spdx: String,
}

#[derive(Debug, Default)]
struct SectionValue {
    crates: BTreeMap<String, HashSet<Rc<AboutLicense>>>,
}

pub fn about_sections() -> &'static BTreeSet<LegalSection> {
    static SECTIONS: OnceLock<BTreeSet<LegalSection>> = OnceLock::new();

    SECTIONS.get_or_init(|| {
        // Collect the license info into a hashmap to deduplicate authors + licenses
        let mut licenses: HashMap<SectionKey, SectionValue> = HashMap::new();
        for license in license_info() {
            let about_license = Rc::new(AboutLicense::from_spdx(
                &license.id,
                &license.name,
                &license.text,
            ));

            for used in &license.used_by {
                let entry = licenses
                    .entry(SectionKey {
                        authors: used.c.authors.clone(),
                        spdx: used.c.license.clone(),
                    })
                    .or_default();

                entry
                    .crates
                    .entry(format!("{} {}", used.c.name, used.c.version))
                    .or_default()
                    .insert(about_license.clone());
            }
        }

        licenses
            .into_iter()
            .flat_map(|(key, value)| {
                value.crates.into_iter().map(move |(c, licenses)| {
                    let licenses = licenses
                        .into_iter()
                        .enumerate()
                        .map(|(i, c)| {
                            if i > 0 {
                                format!("\n\n{c}")
                            } else {
                                c.to_string()
                            }
                        })
                        .collect::<String>();

                    let copyright = if key.authors.is_empty() {
                        None
                    } else {
                        Some(glib::markup_escape_text(&key.authors.join("\n")).to_string())
                    };

                    LegalSection {
                        title: glib::markup_escape_text(&c),
                        copyright,
                        license_type: gtk::License::Custom,
                        license: Some(licenses),
                    }
                })
            })
            .collect::<BTreeSet<_>>()
    })
}

pub trait AboutDialogLicenseExt {
    async fn add_embedded_license_information(&self);
}

impl AboutDialogLicenseExt for adw::AboutDialog {
    async fn add_embedded_license_information(&self) {
        let (sender, receiver) = oneshot::channel();

        gio::spawn_blocking(glib::clone!(move || {
            let res = sender.send(crate::ui::licenses::about_sections());
            if res.is_err() {
                log::error!("channel is closed");
            }
        }));

        let Ok(about_sections) = receiver.await else {
            return;
        };

        let mut peekable = about_sections.iter().peekable();
        let mut titles = Vec::with_capacity(10);

        // Add legal sections asynchronously, layouting them takes quite a bit of time
        glib::idle_add_local_full(
            glib::Priority::LOW,
            glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    if let Some(legal) = peekable.next() {
                        titles.push(legal.title.as_str());

                        if !peekable.peek().is_some_and(|peek| {
                            peek.copyright == legal.copyright
                                && peek.license_type == legal.license_type
                                && peek.license == legal.license
                        }) {
                            dialog.add_legal_section(
                                &titles.join(", "),
                                legal.copyright.as_deref(),
                                legal.license_type,
                                legal.license.as_deref(),
                            );
                            titles.clear();
                        }

                        glib::ControlFlow::Continue
                    } else {
                        log::debug!("Finished adding legal sections to about dialog");
                        glib::ControlFlow::Break
                    }
                }
            ),
        );
    }
}
