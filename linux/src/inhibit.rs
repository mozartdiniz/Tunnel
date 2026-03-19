/// Sleep and idle inhibition.
///
/// On Linux: acquires a systemd-logind `sleep:idle` inhibitor lock via D-Bus.
/// On other platforms: degrades silently to a no-op — the transfer still works.
///
/// Create an `InhibitGuard` at the start of a transfer; drop it when done.

/// RAII guard that holds a sleep inhibitor lock (Linux) or is a no-op (other).
/// When dropped on Linux, the fd is closed and the lock released automatically.
pub struct InhibitGuard {
    #[cfg(target_os = "linux")]
    _fd: Option<zbus::zvariant::OwnedFd>,
}

impl InhibitGuard {
    /// Acquire the inhibitor lock. Falls back to a no-op on any error or on
    /// non-Linux platforms.
    pub async fn acquire(reason: &str) -> Self {
        #[cfg(target_os = "linux")]
        {
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
        #[cfg(not(target_os = "linux"))]
        {
            let _ = reason;
            Self {}
        }
    }
}

#[cfg(target_os = "linux")]
async fn try_acquire(reason: &str) -> anyhow::Result<zbus::zvariant::OwnedFd> {
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

    let (fd,): (zbus::zvariant::OwnedFd,) = reply.body().deserialize()?;
    Ok(fd)
}
