/// TLS identity management.
///
/// Strategy:
///   - On first run: generate a self-signed cert with rcgen and persist it.
///   - On subsequent runs: load from disk.
///   - Peer verification: TOFU (Trust On First Use).
///       First connection from a peer → fingerprint is stored.
///       Subsequent connections → fingerprint must match.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, ServerConfig, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::config::Config;

const CERT_FILE: &str = "cert.der";
const KEY_FILE: &str = "key.der";
const PEERS_FILE: &str = "known_peers.json";

pub struct TlsStack {
    pub acceptor: TlsAcceptor,
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
            let cert = std::fs::read(&cert_path)?;
            let key = std::fs::read(&key_path)?;
            (cert, key)
        } else {
            tracing::info!("Generating new self-signed TLS identity");
            let (cert, key) = generate_self_signed(&config.device_name)?;
            std::fs::write(&cert_path, &cert)?;
            std::fs::write(&key_path, &key)?;
            (cert, key)
        };

        let cert = CertificateDer::from(cert_der.clone());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));

        // ── Server config (accepting incoming connections) ──────────────────
        // We don't require client certs — the handshake JSON handles auth.
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)?;

        // ── Client config (connecting to peers) ─────────────────────────────
        let peers_file = data_dir.join(PEERS_FILE);
        let tofu = Arc::new(TofuVerifier::load(peers_file)?);

        let client_config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(tofu)
            .with_no_client_auth();

        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(server_config)),
            connector: TlsConnector::from(Arc::new(client_config)),
        })
    }
}

/// Generate a self-signed certificate (DER format).
/// Returns (cert_der, key_der).
fn generate_self_signed(device_name: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let subject_alt_names = vec![device_name.to_string(), "localhost".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)?;
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    Ok((cert_der, key_der))
}

// ── TOFU Verifier ─────────────────────────────────────────────────────────────

/// Maps peer "server name" (IP:port string) to its expected SHA-256 cert fingerprint.
type PeerMap = HashMap<String, String>;

#[derive(Debug)]
struct TofuVerifier {
    known: RwLock<PeerMap>,
    file: PathBuf,
}

impl TofuVerifier {
    fn load(file: PathBuf) -> Result<Self> {
        let known = if file.exists() {
            let raw = std::fs::read(&file)?;
            serde_json::from_slice(&raw).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self {
            known: RwLock::new(known),
            file,
        })
    }

    fn fingerprint(cert: &CertificateDer<'_>) -> String {
        let mut h = Sha256::new();
        h.update(cert.as_ref());
        hex::encode(h.finalize())
    }

    fn persist(&self) {
        if let Ok(map) = self.known.read() {
            if let Ok(json) = serde_json::to_vec_pretty(&*map) {
                let _ = std::fs::write(&self.file, json);
            }
        }
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
        let fp = Self::fingerprint(end_entity);
        let key = server_name.to_str().to_string();

        let mut known = self.known.write().unwrap();
        match known.get(&key) {
            Some(stored) if stored == &fp => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(Error::General(
                "TOFU violation: certificate fingerprint changed".into(),
            )),
            None => {
                tracing::info!("TOFU: trusting new peer '{key}' (fp: {fp})");
                known.insert(key, fp);
                drop(known);
                self.persist();
                Ok(ServerCertVerified::assertion())
            }
        }
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
