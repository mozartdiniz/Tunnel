/// Thread-safe TOFU store: maps peer key → expected SHA-256 cert fingerprint.
///
/// Key   = peer's announced fingerprint from UDP DeviceInfo (their stable identity).
/// Value = actual SHA-256 fingerprint of the TLS cert they presented.
///
/// First contact: trust and record. Subsequent contacts: must match stored value.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::Result;
use rustls::Error;

type PeerMap = HashMap<String, String>;

#[derive(Debug)]
pub struct TofuStore {
    known: RwLock<PeerMap>,
    file: PathBuf,
}

impl TofuStore {
    pub fn load(file: PathBuf) -> Result<Self> {
        let known = if file.exists() {
            serde_json::from_slice(&std::fs::read(&file)?).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self { known: RwLock::new(known), file })
    }

    /// Accept on first contact; enforce fingerprint match on all subsequent contacts.
    pub fn verify(&self, key: &str, fingerprint: &str) -> Result<(), Error> {
        let mut known = self.known.write().unwrap();
        match known.get(key) {
            Some(stored) if stored == fingerprint => Ok(()),
            Some(_) => Err(Error::General(
                "TOFU violation: certificate fingerprint changed".into(),
            )),
            None => {
                tracing::info!("TOFU: trusting new peer '{key}' (fp: {fingerprint})");
                known.insert(key.to_string(), fingerprint.to_string());
                drop(known);
                if let Err(e) = self.persist() {
                    tracing::warn!("TOFU: failed to persist known_peers: {e}");
                }
                Ok(())
            }
        }
    }

    fn persist(&self) -> Result<()> {
        let map = self.known.read().unwrap();
        let json = serde_json::to_vec_pretty(&*map)?;
        std::fs::write(&self.file, json)?;
        Ok(())
    }
}
