/// LocalSend peer discovery via UDP multicast + HTTP subnet scan.
///
/// Each device:
///   - Announces itself by sending a DeviceInfo JSON payload to 224.0.0.167:53317.
///   - Listens on the same multicast group for announcements from other devices.
///   - Responds to incoming announcements with its own DeviceInfo so the sender
///     immediately learns about this device.
///
/// Multi-interface note: on machines with both Ethernet and Wi-Fi (or virtual
/// adapters) the OS may join multicast or route outbound packets on the wrong
/// interface, causing discovery to fail across network segments.  We therefore
/// enumerate every active non-loopback IPv4 interface and:
///   - join the multicast group on each one (so we receive from all of them), and
///   - send announcements out of each one (so peers on any adapter see us).
///
/// Subnet scan: when multicast fails (e.g. Ethernet ↔ Wi-Fi on different subnets),
/// `scan_subnets` probes every host in each local /24-or-smaller subnet via HTTPS.
/// Because the router forwards TCP between subnets even when it doesn't forward
/// multicast, this finds peers that multicast misses.
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::localsend::{DeviceInfo, LOCALSEND_PORT, MULTICAST_ADDR};

const HEARTBEAT_INTERVAL_SECS: u64 = 10;
const PEER_EXPIRY_SECS: u64 = 30;

/// Events yielded by the browse task and the subnet scanner.
pub enum DiscoveryEvent {
    PeerFound { fingerprint: String, alias: String, addr: IpAddr, port: u16, sync_enabled: bool },
    PeerLost { fingerprint: String },
}

pub struct Discovery {
    fingerprint: String,
    sync_enabled: Arc<AtomicBool>,
}

impl Discovery {
    pub fn set_sync_enabled(&self, enabled: bool) {
        self.sync_enabled.store(enabled, Ordering::Relaxed);
    }
}

// ── Interface helpers ────────────────────────────────────────────────────────

/// Return (IPv4 address, prefix length) for every active, non-loopback interface.
fn local_ipv4_addrs_with_prefix() -> Vec<(Ipv4Addr, u8)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) => {
                let prefix = u32::from(v4.netmask).count_ones() as u8;
                Some((v4.ip, prefix))
            }
            _ => None,
        })
        .collect()
}

/// Return only the IPv4 addresses (no prefix) of active, non-loopback interfaces.
fn local_ipv4_addrs() -> Vec<Ipv4Addr> {
    local_ipv4_addrs_with_prefix().into_iter().map(|(ip, _)| ip).collect()
}

/// Send `payload` to the multicast group via every active non-loopback IPv4
/// interface.  If no interfaces are found the OS default is used as a fallback.
async fn multicast_send(payload: Vec<u8>) {
    let dest = format!("{MULTICAST_ADDR}:{LOCALSEND_PORT}");
    let addrs = local_ipv4_addrs();

    if addrs.is_empty() {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await {
            let _ = sock.send_to(&payload, &dest).await;
        }
        return;
    }

    for local_addr in addrs {
        let Ok(raw) = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) else {
            continue;
        };
        let _ = raw.set_nonblocking(true);
        let _ = raw.bind(&SocketAddr::from(([0u8, 0, 0, 0], 0u16)).into());
        if raw.set_multicast_if_v4(&local_addr).is_err() {
            continue;
        }
        let Ok(sock) = UdpSocket::from_std(raw.into()) else { continue };
        let payload = payload.clone();
        let dest = dest.clone();
        tokio::spawn(async move {
            let _ = sock.send_to(&payload, dest).await;
        });
    }
}

// ── Subnet scanner ───────────────────────────────────────────────────────────

/// Every host address in each local /24-or-smaller subnet, excluding our own IPs.
/// Subnets wider than /24 are clamped to /24 to avoid scanning thousands of hosts.
fn scan_candidate_ips() -> Vec<Ipv4Addr> {
    let interfaces = local_ipv4_addrs_with_prefix();
    let own: HashSet<Ipv4Addr> = interfaces.iter().map(|(ip, _)| *ip).collect();
    let mut candidates: HashSet<Ipv4Addr> = HashSet::new();

    for (ip, prefix_len) in interfaces {
        let effective_prefix = prefix_len.max(24);
        let host_bits = 32u8.saturating_sub(effective_prefix);
        let ip_u32 = u32::from(ip);
        let mask = (!0u32).checked_shl(host_bits as u32).unwrap_or(0);
        let network = ip_u32 & mask;
        let host_count = (1u32 << host_bits).saturating_sub(2);

        for i in 1..=host_count {
            let addr = Ipv4Addr::from(network | i);
            if !own.contains(&addr) {
                candidates.insert(addr);
            }
        }
    }

    let mut out: Vec<Ipv4Addr> = candidates.into_iter().collect();
    out.sort();
    out
}

/// Probe every host in the local subnets via HTTPS and return `PeerFound` events
/// for each one that responds with a valid LocalSend `DeviceInfo`.
///
/// All probes run concurrently with a 500 ms timeout each, so the scan
/// completes in well under a second even for a full /24.
pub async fn scan_subnets(own_fingerprint: &str) -> Vec<DiscoveryEvent> {
    let Ok(client) = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_millis(500))
        .build()
    else {
        tracing::warn!("scan_subnets: failed to build HTTP client");
        return vec![];
    };

    let candidates = scan_candidate_ips();
    if candidates.is_empty() {
        return vec![];
    }
    tracing::info!("LocalSend: scanning {} candidate IPs", candidates.len());

    let own_fp = own_fingerprint.to_string();
    let tasks: Vec<_> = candidates
        .into_iter()
        .map(|ip| {
            let client = client.clone();
            let own_fp = own_fp.clone();
            tokio::spawn(async move {
                let url =
                    format!("https://{}:{}/api/localsend/v2/info", ip, LOCALSEND_PORT);
                let resp = client.get(&url).send().await.ok()?;
                let info = resp.json::<DeviceInfo>().await.ok()?;
                if info.fingerprint == own_fp || info.alias.len() > 256 {
                    return None;
                }
                tracing::info!(
                    "LocalSend scan: found {} @ {}:{}",
                    info.alias, ip, info.port
                );
                Some(DiscoveryEvent::PeerFound {
                    fingerprint: info.fingerprint,
                    alias: info.alias,
                    addr: IpAddr::V4(ip),
                    port: info.port,
                    sync_enabled: info.sync == Some(true),
                })
            })
        })
        .collect();

    let mut events = Vec::new();
    for task in tasks {
        if let Ok(Some(ev)) = task.await {
            events.push(ev);
        }
    }
    events
}

// ── Discovery (multicast) ────────────────────────────────────────────────────

impl Discovery {
    pub fn new(fingerprint: String) -> Self {
        Self { fingerprint, sync_enabled: Arc::new(AtomicBool::new(false)) }
    }

    /// Send a UDP multicast announcement so other devices can discover us.
    pub async fn advertise(&self, alias: &str, port: u16) -> Result<()> {
        let info = self.build_info(alias, port, Some(true));
        let payload = serde_json::to_vec(&info)?;
        multicast_send(payload).await;
        tracing::info!("LocalSend: announced '{alias}' on port {port}");
        Ok(())
    }

    /// Send a UDP multicast goodbye packet (announce=false).
    pub async fn unregister(&self, alias: &str, port: u16) -> Result<()> {
        let info = self.build_info(alias, port, Some(false));
        let payload = serde_json::to_vec(&info)?;
        multicast_send(payload).await;
        Ok(())
    }

    /// Bind to the multicast port and start a background task that yields
    /// `DiscoveryEvent`s as peers come and go.
    pub async fn browse(&self, alias: String, port: u16) -> Result<mpsc::Receiver<DiscoveryEvent>> {
        let (tx, rx) = mpsc::channel(32);
        let own_fingerprint = self.fingerprint.clone();
        let multicast_ip: Ipv4Addr = MULTICAST_ADDR.parse().unwrap();
        let own_info = self.build_info(&alias, port, None);
        let heartbeat_info = self.build_info(&alias, port, Some(true));

        let std_socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        std_socket.set_reuse_address(true)?;
        std_socket.bind(&SocketAddr::from(([0u8, 0, 0, 0], LOCALSEND_PORT)).into())?;

        // Join the multicast group on every non-loopback IPv4 interface so we
        // receive announcements regardless of which adapter the sender used.
        let local_addrs = local_ipv4_addrs();
        if local_addrs.is_empty() {
            std_socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)?;
        } else {
            let mut joined = false;
            for addr in &local_addrs {
                match std_socket.join_multicast_v4(&multicast_ip, addr) {
                    Ok(_) => {
                        tracing::debug!("Joined multicast on interface {addr}");
                        joined = true;
                    }
                    Err(e) => tracing::warn!("join_multicast on {addr}: {e}"),
                }
            }
            if !joined {
                std_socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)?;
            }
        }

        std_socket.set_nonblocking(true)?;
        let socket = Arc::new(UdpSocket::from_std(std_socket.into())?);

        tokio::spawn(async move {
            let mut peer_last_seen: std::collections::HashMap<String, Instant> =
                std::collections::HashMap::new();
            let mut buf = vec![0u8; 4096];
            let mut expiry_tick =
                tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
            let mut heartbeat_tick =
                tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
            heartbeat_tick.reset();
            let heartbeat_payload = serde_json::to_vec(&heartbeat_info).unwrap_or_default();

            loop {
                tokio::select! {
                    result = socket.recv_from(&mut buf) => {
                        let Ok((n, src)) = result else { break };
                        let Ok(info) = serde_json::from_slice::<DeviceInfo>(&buf[..n]) else {
                            continue;
                        };

                        if info.fingerprint == own_fingerprint { continue; }
                        if info.alias.len() > 256 { continue; }

                        let fp = info.fingerprint.clone();
                        let is_goodbye = info.announce == Some(false);

                        if is_goodbye {
                            if peer_last_seen.remove(&fp).is_some() {
                                let _ = tx
                                    .send(DiscoveryEvent::PeerLost { fingerprint: fp })
                                    .await;
                            }
                            continue;
                        }

                        let was_known = peer_last_seen.contains_key(&fp);
                        peer_last_seen.insert(fp.clone(), Instant::now());

                        if !was_known {
                            tracing::info!(
                                "LocalSend: peer found: {} @ {}:{}",
                                info.alias, src.ip(), info.port
                            );
                            let _ = tx
                                .send(DiscoveryEvent::PeerFound {
                                    fingerprint: fp,
                                    alias: info.alias,
                                    addr: src.ip(),
                                    port: info.port,
                                    sync_enabled: info.sync == Some(true),
                                })
                                .await;

                            let resp = serde_json::to_vec(&own_info).unwrap_or_default();
                            tokio::spawn(multicast_send(resp));
                        }
                    }

                    _ = expiry_tick.tick() => {
                        let expired: Vec<String> = peer_last_seen
                            .iter()
                            .filter(|(_, t)| t.elapsed() > Duration::from_secs(PEER_EXPIRY_SECS))
                            .map(|(k, _)| k.clone())
                            .collect();
                        for fp in expired {
                            peer_last_seen.remove(&fp);
                            tracing::info!("LocalSend: peer timed out: {fp}");
                            let _ = tx
                                .send(DiscoveryEvent::PeerLost { fingerprint: fp })
                                .await;
                        }
                    }

                    _ = heartbeat_tick.tick() => {
                        tokio::spawn(multicast_send(heartbeat_payload.clone()));
                    }
                }
            }
        });

        Ok(rx)
    }

    fn build_info(&self, alias: &str, port: u16, announce: Option<bool>) -> DeviceInfo {
        let sync = if self.sync_enabled.load(Ordering::Relaxed) { Some(true) } else { None };
        DeviceInfo {
            alias: alias.to_string(),
            version: "2.0".to_string(),
            device_model: Some("PC".to_string()),
            device_type: Some("desktop".to_string()),
            fingerprint: self.fingerprint.clone(),
            port,
            protocol: "https".to_string(),
            download: false,
            announce,
            sync,
        }
    }
}
