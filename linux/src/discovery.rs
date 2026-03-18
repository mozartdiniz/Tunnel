/// LocalSend peer discovery via UDP multicast.
///
/// Each device:
///   - Announces itself by sending a DeviceInfo JSON payload to 224.0.0.167:53317.
///   - Listens on the same multicast group for announcements from other devices.
///   - Responds to incoming announcements with its own DeviceInfo so the sender
///     immediately learns about this device.
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::localsend::{DeviceInfo, LOCALSEND_PORT, MULTICAST_ADDR};

/// Events yielded by the browse task.
pub enum DiscoveryEvent {
    PeerFound { fingerprint: String, alias: String, addr: IpAddr, port: u16 },
    PeerLost { fingerprint: String },
}

pub struct Discovery {
    fingerprint: String,
}

impl Discovery {
    pub fn new(fingerprint: String) -> Self {
        Self { fingerprint }
    }

    /// Send a UDP multicast announcement so other devices can discover us.
    /// Returns our own fingerprint (used by app.rs for self-filtering).
    pub async fn advertise(&self, alias: &str, port: u16) -> Result<String> {
        let info = self.build_info(alias, port, Some(true));
        let payload = serde_json::to_vec(&info)?;

        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.send_to(&payload, format!("{MULTICAST_ADDR}:{LOCALSEND_PORT}")).await?;

        tracing::info!("LocalSend: announced '{alias}' on port {port}");
        Ok(self.fingerprint.clone())
    }

    /// Send a UDP multicast goodbye packet (announce=false).
    pub async fn unregister(&self, alias: &str, port: u16) -> Result<()> {
        let info = self.build_info(alias, port, Some(false));
        let payload = serde_json::to_vec(&info)?;

        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.send_to(&payload, format!("{MULTICAST_ADDR}:{LOCALSEND_PORT}")).await?;
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

        // Bind to 0.0.0.0:53317 with SO_REUSEPORT so multiple processes can coexist.
        let std_socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        std_socket.set_reuse_address(true)?;
        std_socket.bind(&SocketAddr::from(([0u8, 0, 0, 0], LOCALSEND_PORT)).into())?;
        std_socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)?;
        std_socket.set_nonblocking(true)?;
        let socket = Arc::new(UdpSocket::from_std(std_socket.into())?);

        tokio::spawn(async move {
            let mut peer_last_seen: HashMap<String, Instant> = HashMap::new();
            let mut buf = vec![0u8; 4096];
            // Re-check for expired peers every 10 seconds.
            let mut expiry_tick = tokio::time::interval(Duration::from_secs(10));
            // Re-announce every 10 seconds so remote peers don't expire us.
            let mut heartbeat_tick = tokio::time::interval(Duration::from_secs(10));
            heartbeat_tick.reset(); // skip the immediate first tick — advertise() already ran
            let heartbeat_payload = serde_json::to_vec(&heartbeat_info).unwrap_or_default();

            loop {
                tokio::select! {
                    result = socket.recv_from(&mut buf) => {
                        let Ok((n, src)) = result else { break };
                        let Ok(info) = serde_json::from_slice::<DeviceInfo>(&buf[..n]) else {
                            continue;
                        };

                        // Filter self
                        if info.fingerprint == own_fingerprint {
                            continue;
                        }

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
                            tracing::info!("LocalSend: peer found: {} @ {}:{}", info.alias, src.ip(), info.port);
                            let _ = tx.send(DiscoveryEvent::PeerFound {
                                fingerprint: fp,
                                alias: info.alias,
                                addr: src.ip(),
                                port: info.port,
                            }).await;

                            // Respond with our own info so the peer immediately discovers us.
                            let resp_payload = serde_json::to_vec(&own_info).unwrap_or_default();
                            let dest = format!("{MULTICAST_ADDR}:{LOCALSEND_PORT}");
                            let sock = socket.clone();
                            tokio::spawn(async move {
                                let _ = sock.send_to(&resp_payload, dest).await;
                            });
                        }
                    }

                    _ = expiry_tick.tick() => {
                        let expired: Vec<String> = peer_last_seen
                            .iter()
                            .filter(|(_, t)| t.elapsed() > Duration::from_secs(30))
                            .map(|(k, _)| k.clone())
                            .collect();
                        for fp in expired {
                            peer_last_seen.remove(&fp);
                            tracing::info!("LocalSend: peer timed out: {fp}");
                            let _ = tx.send(DiscoveryEvent::PeerLost { fingerprint: fp }).await;
                        }
                    }

                    _ = heartbeat_tick.tick() => {
                        let payload = heartbeat_payload.clone();
                        let dest = format!("{MULTICAST_ADDR}:{LOCALSEND_PORT}");
                        let sock = socket.clone();
                        tokio::spawn(async move {
                            let _ = sock.send_to(&payload, dest).await;
                        });
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
