/// TLS identity management.
///
/// Strategy:
///   - On first run: generate a self-signed cert with rcgen and persist it.
///   - On subsequent runs: load from disk.
///   - Peer verification: TOFU (Trust On First Use) on the sender side.
///       The receiver presents a self-signed cert; the sender TOFU-verifies it by fingerprint.
///       No mutual TLS — LocalSend uses one-way HTTPS.
///   - Tunnel enhancement: TOFU fingerprint persistence (open issue #2430 in LocalSend repo).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, ServerConfig, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::config::Config;

const CERT_FILE: &str = "cert.der";
const KEY_FILE: &str = "key.der";
/// TOFU store for server certs we've verified as a client (outgoing connections).
const SERVER_PEERS_FILE: &str = "known_peers.json";

/// Maps peer announced fingerprint → expected TLS cert fingerprint.
type PeerMap = HashMap<String, String>;

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

        // ── TOFU store (shared, persisted) ──────────────────────────────────
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
    /// This ensures the peer's cert matches what they advertised — not just any cert
    /// we've previously seen from any peer.
    pub fn client_config_for_peer(&self, peer_fingerprint: &str) -> ClientConfig {
        let verifier = Arc::new(TargetedTofuVerifier {
            store: self.tofu_store.clone(),
            peer_fp: peer_fingerprint.to_string(),
        });
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

/// Generate a self-signed certificate (DER format). Returns (cert_der, key_der).
fn generate_self_signed(device_name: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let subject_alt_names = vec![device_name.to_string(), "localhost".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)?;
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    Ok((cert_der, key_der))
}

// ── Shared TOFU store ─────────────────────────────────────────────────────────

/// Thread-safe TOFU store: maps peer key → expected SHA-256 cert fingerprint.
#[derive(Debug)]
struct TofuStore {
    known: RwLock<PeerMap>,
    file: PathBuf,
}

impl TofuStore {
    fn load(file: PathBuf) -> Result<Self> {
        let known = if file.exists() {
            serde_json::from_slice(&std::fs::read(&file)?).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self { known: RwLock::new(known), file })
    }

    /// Accept on first contact; enforce fingerprint match on all subsequent contacts.
    fn verify(&self, key: &str, fingerprint: &str) -> Result<(), Error> {
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

// ── Server cert verifier (used by the sending side) ───────────────────────────

/// Verifies the receiver's TLS cert using TOFU keyed by the peer's *announced* fingerprint.
///
/// Key   = peer's fingerprint from UDP DeviceInfo (their stable identity).
/// Value = actual SHA-256 fingerprint of the TLS cert they presented.
///
/// First contact: trust and record. Subsequent contacts: must match stored value.
/// This correctly detects a different peer impersonating someone we already trust.
#[derive(Debug)]
struct TargetedTofuVerifier {
    store: Arc<TofuStore>,
    /// Peer's announced fingerprint (from UDP discovery) — used as the TOFU key.
    peer_fp: String,
}

impl ServerCertVerifier for TargetedTofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let actual_fp = TlsStack::fingerprint(end_entity);
        self.store.verify(&self.peer_fp, &actual_fp)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
