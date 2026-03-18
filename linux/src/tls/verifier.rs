/// Custom rustls `ServerCertVerifier` implementing TOFU for outgoing connections.
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};

use super::tofu_store::TofuStore;

/// Verifies the receiver's TLS cert using TOFU keyed by the peer's *announced* fingerprint.
///
/// This correctly detects a different peer impersonating someone we already trust.
#[derive(Debug)]
pub struct TargetedTofuVerifier {
    pub(super) store: Arc<TofuStore>,
    /// Peer's announced fingerprint (from UDP discovery) — used as the TOFU key.
    pub(super) peer_fp: String,
}

impl TargetedTofuVerifier {
    pub fn new(store: Arc<TofuStore>, peer_fp: String) -> Self {
        Self { store, peer_fp }
    }
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
        let mut h = Sha256::new();
        h.update(end_entity.as_ref());
        let actual_fp = hex::encode(h.finalize());
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
