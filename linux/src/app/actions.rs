/// Application-level GAction names.
///
/// `strum::AsRefStr` derives the kebab-case string from each variant, so
/// action names are never hand-written string literals — a typo becomes a
/// compile error. Use `.as_ref()` for `SimpleAction::new` (bare name) and
/// `.prefixed()` for notification buttons and `activate_action` ("app.<name>").
#[derive(strum::AsRefStr)]
#[strum(serialize_all = "kebab-case")]
pub enum AppAction {
    AcceptTransfer,
    DenyTransfer,
    RevealFile,
}

impl AppAction {
    /// Returns the full action name with the `"app."` prefix, e.g.
    /// `"app.accept-transfer"`. Use this in notification buttons and
    /// `gio::Application::activate_action`.
    pub fn prefixed(&self) -> String {
        format!("app.{}", self.as_ref())
    }
}
