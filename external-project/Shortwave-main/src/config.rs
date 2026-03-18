use std::sync::LazyLock;

/// Declares a lazily-initialized static variable that reads its value from a
/// compile-time environment variable with a `MESON_` prefix.
///
/// Panics on first access if the environment variable was not set during compilation.
macro_rules! config_var {
    ($name:ident) => {
        #[expect(clippy::option_env_unwrap)]
        pub static $name: LazyLock<&'static str> = LazyLock::new(|| {
            option_env!(concat!("MESON_", stringify!($name))).expect(concat!(
                "MESON_",
                stringify!($name),
                " was not set at compile time"
            ))
        });
    };
}

config_var!(NAME);
config_var!(PKGNAME);
config_var!(APP_ID);
config_var!(PATH_ID);
config_var!(VERSION);
config_var!(PROFILE);
config_var!(VCS_TAG);
config_var!(LOCALEDIR);
config_var!(DATADIR);
