/// LocalSend peer discovery via UDP multicast.
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
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::localsend::{DeviceInfo, LOCALSEND_PORT, MULTICAST_ADDR};

const HEARTBEAT_INTERVAL_SECS: u64 = 10;
const PEER_EXPIRY_SECS: u64 = 30;

/// Events yielded by the browse task.
pub enum DiscoveryEvent {
    PeerFound { fingerprint: String, alias: String, addr: IpAddr, port: u16 },
    PeerLost { fingerprint: String },
}

pub struct Discovery {
    fingerprint: String,
}

// ── Interface helpers ────────────────────────────────────────────────────────

/// Return the IPv4 address of every active, non-loopback network interface.
/// Falls back to an empty Vec on any error; callers should treat that as
/// "let the OS decide" (i.e. bind to INADDR_ANY / 0.0.0.0).
fn local_ipv4_addrs() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) => Some(v4.ip),
            _ => None,
        })
        .collect()
}

/// Send `payload` to the multicast group via every active non-loopback IPv4
/// interface.  If no interfaces are found the OS default is used as a fallback.
async fn multicast_send(payload: Vec<u8>) {
    let dest = format!("{MULTICAST_ADDR}:{LOCALSEND_PORT}");
    let addrs = local_ipv4_addrs();

    if addrs.is_empty() {
        // Fallback: let the OS choose the outbound interface.
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

// ── Discovery ────────────────────────────────────────────────────────────────

impl Discovery {
    pub fn new(fingerprint: String) -> Self {
        Self { fingerprint }
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
        // None = "I'm here, responding" — distinct from Some(false) which means goodbye.
        let own_info = self.build_info(&alias, port, None);
        // Heartbeat: re-announce every 10 s so remote peers don't expire us.
        let heartbeat_info = self.build_info(&alias, port, Some(true));

        // Bind to 0.0.0.0:53317 with SO_REUSEADDR so multiple processes can coexist.
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
            // If every per-interface join failed, fall back to INADDR_ANY.
            if !joined {
                std_socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)?;
            }
        }

        std_socket.set_nonblocking(true)?;
        let socket = Arc::new(UdpSocket::from_std(std_socket.into())?);

        tokio::spawn(async move {
            let mut peer_last_seen: HashMap<String, Instant> = HashMap::new();
            let mut buf = vec![0u8; 4096];
            let mut expiry_tick =
                tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
            let mut heartbeat_tick =
                tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
            heartbeat_tick.reset(); // skip immediate first tick — advertise() already ran
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
                                let _ = tx.send(DiscoveryEvent::PeerLost { fingerprint: fp }).await;
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
                                })
                                .await;

                            // Respond with our own info so the peer immediately discovers us.
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
        }
    }
}
