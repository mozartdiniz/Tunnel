/// TLS identity management.
///
/// Strategy:
///   - On first run: generate a self-signed cert with rcgen and persist it.
///   - On subsequent runs: load from disk.
///   - Peer verification: mutual TOFU (Trust On First Use) on both sides.
///       Outgoing: we verify the receiver's server cert.
///       Incoming: we verify the sender's client cert (per-connection verifier).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use tokio_rustls::TlsConnector;

use crate::config::Config;

const CERT_FILE: &str = "cert.der";
const KEY_FILE: &str = "key.der";
/// TOFU store for server certs we've verified as a client (outgoing connections).
const SERVER_PEERS_FILE: &str = "known_peers.json";
/// TOFU store for client certs we've verified as a server (incoming connections).
const CLIENT_PEERS_FILE: &str = "known_clients.json";

/// Maps peer key (IP string) to its expected SHA-256 cert fingerprint.
type PeerMap = HashMap<String, String>;

pub struct TlsStack {
    pub cert: CertificateDer<'static>,
    /// Raw key bytes — kept so we can build a fresh ServerConfig per incoming connection.
    key_bytes: Vec<u8>,
    /// Shared TOFU store for client certs (incoming connections, keyed by peer IP).
    client_tofu: Arc<TofuStore>,
    /// Outgoing connector — presents our cert + verifies server cert via TOFU.
    pub connector: TlsConnector,
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

        let cert = CertificateDer::from(cert_der.clone());
        let key_bytes = key_der.clone();

        // ── Client config (outgoing) ─────────────────────────────────────────
        // Present our cert so the receiver can verify us, and verify the
        // receiver's cert via TOFU.
        let server_tofu = Arc::new(TofuVerifier::load(data_dir.join(SERVER_PEERS_FILE))?);
        let key_for_client = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));
        let client_config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(server_tofu)
            .with_client_auth_cert(vec![cert.clone()], key_for_client)?;

        // ── Shared TOFU store for incoming client certs ──────────────────────
        let client_tofu = Arc::new(TofuStore::load(data_dir.join(CLIENT_PEERS_FILE))?);

        Ok(Self {
            cert,
            key_bytes,
            client_tofu,
            connector: TlsConnector::from(Arc::new(client_config)),
        })
    }

    /// Build a TLS acceptor that requires and TOFU-verifies the connecting client's cert.
    /// Call once per accepted TCP connection (peer IP is baked into the verifier).
    pub fn make_acceptor_for_peer(&self, peer_ip: &str) -> Result<tokio_rustls::TlsAcceptor> {
        let verifier = Arc::new(TofuClientVerifier {
            peer_key: peer_ip.to_string(),
            store: Arc::clone(&self.client_tofu),
        });
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_bytes.clone()));
        let server_config = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![self.cert.clone()], key)?;
        Ok(tokio_rustls::TlsAcceptor::from(Arc::new(server_config)))
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
                self.persist();
                Ok(())
            }
        }
    }

    fn persist(&self) {
        if let Ok(map) = self.known.read() {
            if let Ok(json) = serde_json::to_vec_pretty(&*map) {
                let _ = std::fs::write(&self.file, json);
            }
        }
    }
}

// ── Server cert verifier (used by the sending side) ───────────────────────────

/// Verifies the receiver's server cert when we connect outward. TOFU keyed by server IP.
#[derive(Debug)]
struct TofuVerifier {
    store: TofuStore,
}

impl TofuVerifier {
    fn load(file: PathBuf) -> Result<Self> {
        Ok(Self { store: TofuStore::load(file)? })
    }
}

impl ServerCertVerifier for TofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let fp = TlsStack::fingerprint(end_entity);
        let key = server_name.to_str().to_string();
        self.store.verify(&key, &fp)?;
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

// ── Client cert verifier (used by the receiving side) ─────────────────────────

/// Verifies the sender's client cert on an incoming connection. TOFU keyed by peer IP.
/// Created fresh per connection so the peer IP is baked in at construction time.
#[derive(Debug)]
struct TofuClientVerifier {
    peer_key: String,
    store: Arc<TofuStore>,
}

impl ClientCertVerifier for TofuClientVerifier {
    fn client_auth_mandatory(&self) -> bool {
        true // Reject any connection that does not present a certificate.
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[] // We accept any self-signed cert; TOFU handles trust.
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        let fp = TlsStack::fingerprint(end_entity);
        self.store.verify(&self.peer_key, &fp)?;
        Ok(ClientCertVerified::assertion())
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
