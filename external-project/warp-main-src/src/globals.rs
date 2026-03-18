use regex::Regex;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

pub static WORMHOLE_DEFAULT_RENDEZVOUS_SERVER_STR: &str = "ws://relay.magic-wormhole.io:4000";
pub static WORMHOLE_DEFAULT_RENDEZVOUS_SERVER: LazyLock<url::Url> =
    LazyLock::new(|| url::Url::parse(WORMHOLE_DEFAULT_RENDEZVOUS_SERVER_STR).unwrap());

pub static WORMHOLE_DEFAULT_TRANSIT_RELAY_URL_STR: &str = "tcp://transit.magic-wormhole.io:4001";
pub static WORMHOLE_DEFAULT_TRANSIT_RELAY_HINTS: LazyLock<Vec<wormhole::transit::RelayHint>> =
    LazyLock::new(|| {
        vec![
            wormhole::transit::RelayHint::from_urls(
                None,
                [WORMHOLE_DEFAULT_TRANSIT_RELAY_URL_STR.parse().unwrap()],
            )
            .unwrap(),
        ]
    });

pub const WORMHOLE_DEFAULT_APPID_STR: &str = "lothar.com/wormhole/text-or-file-xfer";

pub static TRANSMIT_URI_FIND_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"wormhole-transfer:\d+-\S+").unwrap());
pub static TRANSMIT_CODE_FIND_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+-\S+").unwrap());
pub static TRANSMIT_CODE_MATCH_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d+-\S+$").unwrap());

#[cfg(debug_assertions)]
pub const DEBUG_BUILD: bool = true;
#[cfg(not(debug_assertions))]
pub const DEBUG_BUILD: bool = false;

pub const APP_ID: &str = if DEBUG_BUILD {
    "app.drey.Warp.Devel"
} else {
    "app.drey.Warp"
};

pub static PANIC_BACKTRACES: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Default::default);

pub const APP_NAME: &str = "warp";
pub const GETTEXT_PACKAGE: &str = APP_NAME;
pub const DEFAULT_LOCALEDIR_LINUX: &str = "/usr/share/locale";
pub const PKGDATADIR: &str = "/app/share/warp";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub static CACHE_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = glib::user_cache_dir();
    path.push(APP_ID);
    path
});
/// On Windows, resources are packaged in a common folder next to the executable.
/// If the current exe cannot be found, fall back to the working directory (".") instead.
pub static WINDOWS_BASE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::current_exe().map_or_else(
        |_| ".".into(),
        |mut exe| {
            exe.pop();
            exe
        },
    )
});

pub static GRESOURCE_DATA: &[u8] =
    gvdb_macros::include_gresource_from_dir!("/app/drey/Warp", "data/resources");
