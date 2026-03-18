use crate::error::{AppError, UiError};
use crate::gettext::gettextf;
use crate::gettext::*;
use crate::globals;
use crate::globals::{TRANSMIT_CODE_FIND_REGEX, TRANSMIT_URI_FIND_REGEX};
use gio::prelude::*;
use std::fmt::Display;
use std::str::FromStr;
use wormhole::Code;

pub mod error;
pub mod future;
pub mod zip;

pub async fn show_dir(path: &std::path::Path) -> Result<(), AppError> {
    #[cfg(target_os = "linux")]
    {
        show_dir_dbus(path).await
    }
    #[cfg(target_os = "windows")]
    {
        show_dir_windows(path)
    }
}

#[cfg(target_os = "windows")]
fn show_dir_windows(path: &std::path::Path) -> Result<(), AppError> {
    let cmd = "explorer.exe";
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(path.as_os_str());

    log::debug!("Running command '{} {}'", cmd, arg.to_string_lossy(),);

    std::process::Command::new(cmd)
        .arg(arg)
        .output()
        .map_err(|_| UiError::new(&gettext("Failed to open downloads folder.")))?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn show_dir_dbus(path: &std::path::Path) -> Result<(), AppError> {
    let err_msg = gettext("Failed to open downloads folder.");

    let file = gio::File::for_path(path);
    let window = crate::ui::application::WarpApplication::default().main_window();

    if smol::fs::metadata(path).await?.is_dir() {
        // If this is a directory, we can open it with `FileLauncher::launch`, as the default file handler for
        // directories is the file browser itself.
        // If we'd use "OpenDirectory" instead, it would open the parent and pre-select the dir, not show the dir itself
        gtk::FileLauncher::new(Some(&file))
            .launch_future(Some(&window))
            .await
    } else {
        // If this is a file we want to be displaying, open the containing folder in file browser, and pre-select the file
        gtk::FileLauncher::new(Some(&file))
            .open_containing_folder_future(Some(&window))
            .await
    }
    .map_err(|_| UiError::new(&err_msg))?;

    Ok(())
}

pub fn extract_transmit_uri(str: &str) -> Option<String> {
    TRANSMIT_URI_FIND_REGEX
        .find(str)
        .map(|m| m.as_str().to_owned())
}

pub fn extract_transmit_code(str: &str) -> Option<Code> {
    TRANSMIT_CODE_FIND_REGEX
        .find(str)
        .and_then(|m| m.as_str().parse().ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferDirection {
    Send,
    Receive,
}

impl Default for TransferDirection {
    fn default() -> Self {
        Self::Send
    }
}

#[derive(Debug)]
pub struct WormholeURIParseError(String);

impl Display for WormholeURIParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<wormhole::ParseCodeError> for WormholeURIParseError {
    fn from(value: wormhole::ParseCodeError) -> Self {
        Self(AppError::from(value).gettext_error())
    }
}

#[derive(Debug)]
pub struct WormholeTransferURI {
    pub code: wormhole::Code,
    pub version: usize,
    pub rendezvous_server: url::Url,
    pub direction: TransferDirection,
}

impl WormholeTransferURI {
    pub fn new(
        code: wormhole::Code,
        rendezvous_server: url::Url,
        direction: TransferDirection,
    ) -> Self {
        let mut rendezvous_server = rendezvous_server;
        rendezvous_server.set_path("");
        rendezvous_server.set_query(None);

        Self {
            code,
            version: 0,
            rendezvous_server,
            direction,
        }
    }

    pub fn create_uri(&self) -> String {
        let mut uri = url::Url::parse(&format!(
            "wormhole-transfer:{}",
            urlencoding::encode(self.code.as_ref())
        ))
        .unwrap();

        if self.version != 0 {
            uri.query_pairs_mut()
                .append_pair("version", &self.version.to_string());
        }

        // We take the default here, not the current config. Any non-default should be in the uri.
        let mut rendezvous_server = self.rendezvous_server.clone();
        rendezvous_server.set_path("");
        rendezvous_server.set_query(None);

        if rendezvous_server != *globals::WORMHOLE_DEFAULT_RENDEZVOUS_SERVER {
            uri.query_pairs_mut()
                .append_pair("rendezvous", rendezvous_server.as_ref());
        }

        if self.direction != TransferDirection::Receive {
            uri.query_pairs_mut().append_pair("role", "leader");
        }

        uri.to_string()
    }

    /// This assumes the rendezvous server URI inside the `AppConfig` is a valid URI
    pub fn from_app_cfg_with_code_direction(
        app_cfg: &wormhole::AppConfig<wormhole::transfer::AppVersion>,
        code: wormhole::Code,
        direction: TransferDirection,
    ) -> Self {
        let rendezvous_server = url::Url::parse(&app_cfg.rendezvous_url).unwrap();
        Self {
            code,
            version: 0,
            rendezvous_server,
            direction,
        }
    }

    pub fn to_app_cfg(&self) -> wormhole::AppConfig<wormhole::transfer::AppVersion> {
        let mut rendezvous_url = self.rendezvous_server.clone();
        rendezvous_url.set_path("v1");

        wormhole::AppConfig {
            id: wormhole::AppID::new(globals::WORMHOLE_DEFAULT_APPID_STR),
            rendezvous_url: rendezvous_url.to_string().into(),
            app_version: wormhole::transfer::AppVersion::default(),
        }
    }

    pub fn to_paintable_qr(&self) -> gdk::Paintable {
        let uri = if cfg!(feature = "demo") {
            "https://apps.gnome.org/Warp".to_owned()
        } else {
            self.create_uri()
        };

        let qr = qrcode::QrCode::new(uri).unwrap();

        let svg = qr
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(800, 800)
            .light_color(qrcode::render::svg::Color("#0000"))
            .build();
        gdk::Texture::from_bytes(&svg.as_bytes().into())
            .unwrap()
            .upcast()
    }
}

impl TryFrom<url::Url> for WormholeTransferURI {
    type Error = WormholeURIParseError;

    fn try_from(uri: url::Url) -> Result<Self, Self::Error> {
        // Basic validation
        if uri.scheme() != "wormhole-transfer"
            || uri.has_host()
            || uri.has_authority()
            || uri.path() == ""
        {
            return Err(WormholeURIParseError(gettext("The URI format is invalid")));
        }

        let code = urlencoding::decode(uri.path()).map_err(|_| {
            WormholeURIParseError(gettext("The code does not match the required format"))
        })?;
        if !globals::TRANSMIT_CODE_MATCH_REGEX.is_match(&code) {
            return Err(WormholeURIParseError(gettext(
                "The code does not match the required format",
            )));
        }

        let code: wormhole::Code = code.as_ref().parse()?;

        let mut this = WormholeTransferURI::new(
            code,
            globals::WORMHOLE_DEFAULT_RENDEZVOUS_SERVER.clone(),
            TransferDirection::Receive,
        );

        for (field, value) in uri.query_pairs() {
            match &*field {
                "version" => {
                    this.version = {
                        let value_num = value.parse().map_err(|_| {
                            WormholeURIParseError(gettextf("Unknown URI version: {}", &[&value]))
                        })?;
                        if value_num == 0 {
                            value_num
                        } else {
                            return Err(WormholeURIParseError(gettextf(
                                "Unknown URI version: {}",
                                &[&value],
                            )));
                        }
                    }
                }
                "rendezvous" => {
                    this.rendezvous_server = url::Url::parse(&value).map_err(|_| {
                        WormholeURIParseError(gettextf(
                            "The URI parameter “rendezvous” contains an invalid URL: “{}”",
                            &[&value],
                        ))
                    })?;
                }
                "role" => {
                    this.direction = if value == "follower" {
                        TransferDirection::Receive
                    } else if value == "leader" {
                        TransferDirection::Send
                    } else {
                        return Err(WormholeURIParseError(gettextf(
                            "The URI parameter “role” must be “follower” or “leader” (was: “{}”)",
                            &[&value],
                        )));
                    }
                }
                _ => {
                    return Err(WormholeURIParseError(gettextf(
                        "Unknown URI parameter “{}”",
                        &[&field],
                    )));
                }
            }
        }

        Ok(this)
    }
}

impl FromStr for WormholeTransferURI {
    type Err = WormholeURIParseError;

    fn from_str(uri_str: &str) -> Result<Self, Self::Err> {
        let uri = url::Url::parse(uri_str)
            .map_err(|_| WormholeURIParseError(gettext("The URI format is invalid")))?;
        Self::try_from(uri)
    }
}

#[cfg(test)]
mod test {
    use crate::globals;
    use crate::util::{TransferDirection, WormholeTransferURI};

    #[test]
    fn test_create_uri() {
        let params1 = WormholeTransferURI::new(
            "4-hurricane-equipment".parse().unwrap(),
            globals::WORMHOLE_DEFAULT_RENDEZVOUS_SERVER.clone(),
            TransferDirection::Receive,
        );
        assert_eq!(
            params1.create_uri(),
            "wormhole-transfer:4-hurricane-equipment"
        );

        let params2 = WormholeTransferURI::new(
            "8-🙈-🙉-🙊".parse().unwrap(),
            globals::WORMHOLE_DEFAULT_RENDEZVOUS_SERVER.clone(),
            TransferDirection::Receive,
        );
        assert_eq!(
            params2.create_uri(),
            "wormhole-transfer:8-%F0%9F%99%88-%F0%9F%99%89-%F0%9F%99%8A"
        );

        let mut params3 = WormholeTransferURI::new(
            "8-🙈-🙉-🙊".parse().unwrap(),
            url::Url::parse("ws://localhost:4000").unwrap(),
            TransferDirection::Send,
        );
        params3.version = 1;

        assert_eq!(
            params3.create_uri(),
            "wormhole-transfer:8-%F0%9F%99%88-%F0%9F%99%89-%F0%9F%99%8A?version=1&rendezvous=ws%3A%2F%2Flocalhost%3A4000%2F&role=leader"
        );

        // Version != 0 would result in parse error
        params3.version = 0;

        let parsed_params3 = params3.create_uri().parse::<WormholeTransferURI>().unwrap();
        assert_eq!(params3.rendezvous_server, parsed_params3.rendezvous_server);
        assert_eq!(params3.version, parsed_params3.version);
        assert_eq!(params3.direction, parsed_params3.direction);
    }
}
