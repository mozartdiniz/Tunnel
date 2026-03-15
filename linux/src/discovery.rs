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
use std::net::{IpAddr, Ipv4Addr, UdpSocket};

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
    /// Returns the mDNS fullname so the caller can filter out self-discovery.
    pub fn advertise(&self, display_name: &str, port: u16) -> Result<String> {
        let instance_name = sanitize_instance_name(display_name);
        let hostname = format!("{instance_name}.local.");
        let fullname = format!("{instance_name}.{SERVICE_TYPE}");

        let mut properties = HashMap::new();
        properties.insert("display_name".to_string(), display_name.to_string());

        let local_ip = outbound_ipv4()
            .ok_or_else(|| anyhow::anyhow!("Could not determine local IPv4 address"))?;

        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &hostname,
            IpAddr::V4(local_ip),
            port,
            properties,
        )?;

        self.daemon.register(service)?;
        tracing::info!("mDNS: advertising '{display_name}' on {local_ip}:{port}");
        Ok(fullname)
    }

    /// Remove our mDNS announcement (used before re-advertising with a new name).
    pub fn unregister(&self, fullname: &str) -> Result<()> {
        self.daemon.unregister(fullname)?;
        Ok(())
    }

    /// Returns a channel that yields `ServiceEvent`s as peers come and go.
    pub fn browse(&self) -> Result<mdns_sd::Receiver<ServiceEvent>> {
        let receiver = self.daemon.browse(SERVICE_TYPE)?;
        tracing::info!("mDNS: browsing for {SERVICE_TYPE}");
        Ok(receiver)
    }
}

/// Detect the local outbound IPv4 by routing a UDP socket toward a public IP.
/// No packet is actually sent — the OS just picks the right interface.
fn outbound_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip),
        _ => None,
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
