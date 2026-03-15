/// mDNS service discovery.
///
/// Each Tunnel instance:
///   - Advertises itself as `_tunnel-p2p._tcp.local.`
///   - Browses for other instances on the same LAN.
///
/// The `display_name` property carries the human-readable device name.
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;

const SERVICE_TYPE: &str = "_tunnel-p2p._tcp.local.";

pub struct Discovery {
    daemon: ServiceDaemon,
}

impl Discovery {
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        Ok(Self { daemon })
    }

    /// Announce this device on the LAN.
    /// `port` is the TCP port our TLS listener is bound to.
    pub fn advertise(&self, display_name: &str, port: u16) -> Result<()> {
        // Instance name must be unique — use display_name as base.
        // The full service name will be "<instance>._tunnel-p2p._tcp.local."
        let instance_name = sanitize_instance_name(display_name);

        // Hostname for mDNS (must end with ".local.")
        let hostname = format!("{instance_name}.local.");

        let mut properties = HashMap::new();
        properties.insert("display_name".to_string(), display_name.to_string());

        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &hostname,
            "", // empty = let mdns-sd pick the local IP
            port,
            properties,
        )?;

        self.daemon.register(service)?;
        tracing::info!("mDNS: advertising '{display_name}' on port {port}");
        Ok(())
    }

    /// Returns a channel that yields `ServiceEvent`s as peers come and go.
    pub fn browse(&self) -> Result<mdns_sd::Receiver<ServiceEvent>> {
        let receiver = self.daemon.browse(SERVICE_TYPE)?;
        tracing::info!("mDNS: browsing for {SERVICE_TYPE}");
        Ok(receiver)
    }
}

/// Strip characters that are invalid in mDNS instance names.
fn sanitize_instance_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase()
}
