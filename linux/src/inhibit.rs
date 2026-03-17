/// Sleep and idle inhibition via systemd-logind (roadmap 3.4).
///
/// Create an `InhibitGuard` at the start of a transfer; drop it when done.
/// If logind is unreachable (no systemd, Flatpak without the inhibit portal,
/// etc.) the guard degrades silently to a no-op — the transfer still works.
use zbus::zvariant::OwnedFd;

/// RAII guard that holds a systemd-logind `sleep:idle` inhibitor lock.
/// When dropped, the fd is closed and the lock released automatically.
pub struct InhibitGuard {
    _fd: Option<OwnedFd>,
}

impl InhibitGuard {
    /// Acquire the inhibitor lock.  Falls back to a no-op on any error.
    pub async fn acquire(reason: &str) -> Self {
        match try_acquire(reason).await {
            Ok(fd) => {
                tracing::debug!("Sleep inhibitor acquired");
                Self { _fd: Some(fd) }
            }
            Err(e) => {
                tracing::debug!("Sleep inhibition unavailable: {e}");
                Self { _fd: None }
            }
        }
    }

}

async fn try_acquire(reason: &str) -> anyhow::Result<OwnedFd> {
    let conn = zbus::Connection::system().await?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;

    let reply = proxy
        .call_method("Inhibit", &("sleep:idle", "Tunnel", reason, "block"))
        .await?;

    let (fd,): (OwnedFd,) = reply.body().deserialize()?;
    Ok(fd)
}
