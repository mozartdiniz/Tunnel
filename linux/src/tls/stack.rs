/// TLS identity: self-signed cert generation, loading, and per-peer client configs.
///
/// Strategy:
///   - On first run: generate a self-signed cert with rcgen and persist it.
///   - On subsequent runs: load from disk.
///   - Peer verification: TOFU (Trust On First Use) on the sender side.
use std::sync::Arc;

use anyhow::Result;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, ServerConfig};
use sha2::{Digest, Sha256};

use crate::config::Config;

use super::tofu_store::TofuStore;
use super::verifier::TargetedTofuVerifier;

const CERT_FILE: &str = "cert.der";
const KEY_FILE: &str = "key.der";
/// TOFU store for server certs we've verified as a client (outgoing connections).
const SERVER_PEERS_FILE: &str = "known_peers.json";

pub struct TlsStack {
    pub cert: CertificateDer<'static>,
    /// Raw key bytes — kept to build ServerConfig on demand.
    key_bytes: Vec<u8>,
    /// Shared TOFU store (peer announced fp → actual cert fp).
    tofu_store: Arc<TofuStore>,
}

impl TlsStack {
    pub async fn load_or_create(config: &Config) -> Result<Self> {
        let data_dir = Config::data_dir()?;
        std::fs::create_dir_all(&data_dir)?;

        let cert_path = data_dir.join(CERT_FILE);
        let key_path = data_dir.join(KEY_FILE);

        let (cert_der, key_der) = if cert_path.exists() && key_path.exists() {
            tracing::debug!("Loading existing TLS identity from disk");
            (std::fs::read(&cert_path)?, std::fs::read(&key_path)?)
        } else {
            tracing::info!("Generating new self-signed TLS identity");
            let (cert, key) = generate_self_signed(&config.device_name)?;
            std::fs::write(&cert_path, &cert)?;
            std::fs::write(&key_path, &key)?;
            (cert, key)
        };

        let cert = CertificateDer::from(cert_der);
        let key_bytes = key_der;

        let tofu_store = Arc::new(TofuStore::load(data_dir.join(SERVER_PEERS_FILE))?);

        Ok(Self { cert, key_bytes, tofu_store })
    }

    /// Build a one-way TLS ServerConfig (receiver presents cert; no client cert required).
    pub fn make_server_config(&self) -> Result<ServerConfig> {
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_bytes.clone()));
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![self.cert.clone()], key)?;
        Ok(config)
    }

    /// Build a per-peer client config for outgoing HTTPS.
    ///
    /// The verifier keys TOFU by `peer_fingerprint` (the peer's announced identity
    /// from UDP discovery), mapping it to the actual TLS cert fingerprint they present.
    pub fn client_config_for_peer(&self, peer_fingerprint: &str) -> ClientConfig {
        let verifier = Arc::new(TargetedTofuVerifier::new(
            self.tofu_store.clone(),
            peer_fingerprint.to_string(),
        ));
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth()
    }

    /// Full SHA-256 fingerprint of a DER certificate (64 hex chars).
    pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
        let mut h = Sha256::new();
        h.update(cert.as_ref());
        hex::encode(h.finalize())
    }
}

fn generate_self_signed(device_name: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let subject_alt_names = vec![device_name.to_string(), "localhost".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)?;
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    Ok((cert_der, key_der))
}
