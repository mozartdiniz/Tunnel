import Foundation
import Network
import Security
import CryptoKit
import X509
import SwiftASN1

// MARK: - TLSManager

/// Manages the local TLS identity (self-signed cert) and TOFU peer verification.
///
/// LocalSend v2 uses one-way HTTPS:
///   - The receiver (server) presents a self-signed cert.
///   - The sender (client) TOFU-verifies the cert fingerprint via URLSessionDelegate.
///   - No mutual TLS — the client does NOT present a certificate.
actor TLSManager {

    private let identity: SecIdentity
    private(set) var localFingerprint: String
    private var knownPeers: [String: String]  // fingerprint → fingerprint (TOFU store)
    private let peersFileURL: URL

    // MARK: - Bootstrap

    static func loadOrCreate(config: Config) async throws -> TLSManager {
        let dir = Config.dataDir()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        let p12URL     = dir.appendingPathComponent("identity.p12")
        let certDERURL = dir.appendingPathComponent("cert.der")
        let peersURL   = dir.appendingPathComponent("known_peers.json")

        let identity: SecIdentity
        let fingerprint: String
        if FileManager.default.fileExists(atPath: p12URL.path) {
            identity = try importP12(at: p12URL)
            fingerprint = Self.fingerprintOf(identity)
        } else {
            (identity, fingerprint) = try generateAndSave(
                deviceName: config.deviceName,
                p12URL: p12URL,
                certDERURL: certDERURL
            )
        }

        let knownPeers: [String: String]
        if let data = try? Data(contentsOf: peersURL),
           let map = try? JSONDecoder().decode([String: String].self, from: data) {
            knownPeers = map
        } else {
            knownPeers = [:]
        }

        return TLSManager(identity: identity, fingerprint: fingerprint,
                          knownPeers: knownPeers, peersFileURL: peersURL)
    }

    private init(identity: SecIdentity, fingerprint: String,
                 knownPeers: [String: String], peersFileURL: URL) {
        self.identity = identity
        self.localFingerprint = fingerprint
        self.knownPeers = knownPeers
        self.peersFileURL = peersFileURL
    }

    // MARK: - NWParameters (HTTPS server / receiver side)

    /// Parameters for NWListener: server presents cert, no client cert required (one-way TLS).
    func listenerParameters() -> NWParameters {
        let tlsOptions = NWProtocolTLS.Options()
        let capturedIdentity = identity

        sec_protocol_options_set_local_identity(
            tlsOptions.securityProtocolOptions,
            sec_identity_create(capturedIdentity)!
        )
        // One-way TLS: do NOT require the connecting client to present a certificate.
        sec_protocol_options_set_peer_authentication_required(
            tlsOptions.securityProtocolOptions, false
        )

        return NWParameters(tls: tlsOptions, tcp: NWProtocolTCP.Options())
    }

    // MARK: - TOFU for outgoing URLSession connections (sender side)

    /// Verify a peer's cert fingerprint using TOFU.
    /// Returns true on first contact (and stores the fingerprint) or when the fingerprint matches.
    /// Returns false on a TOFU violation (fingerprint changed).
    func verifyCertFingerprint(_ fingerprint: String) -> Bool {
        if let stored = knownPeers[fingerprint] {
            return stored == fingerprint
        }
        // First contact — trust and remember.
        knownPeers[fingerprint] = fingerprint
        persistPeers()
        return true
    }

    // MARK: - Persistence

    private func persistPeers() {
        guard let data = try? JSONEncoder().encode(knownPeers) else { return }
        try? data.write(to: peersFileURL)
    }

    // MARK: - Generation

    private static func generateAndSave(
        deviceName: String,
        p12URL: URL,
        certDERURL: URL
    ) throws -> (SecIdentity, String) {
        // 1. Generate P-256 key + self-signed cert
        let privateKey = P256.Signing.PrivateKey()
        let swiftKey   = try Certificate.PrivateKey(privateKey)
        let name       = try DistinguishedName { CommonName(deviceName) }
        let now        = Date()
        let cert = try Certificate(
            version: .v3,
            serialNumber: Certificate.SerialNumber(),
            publicKey: swiftKey.publicKey,
            notValidBefore: now,
            notValidAfter: Calendar.current.date(byAdding: .year, value: 10, to: now)!,
            issuer: name, subject: name,
            signatureAlgorithm: .ecdsaWithSHA256,
            extensions: try Certificate.Extensions {
                SubjectAlternativeNames([.dnsName(deviceName), .dnsName("localhost")])
                try ExtendedKeyUsage([.serverAuth, .clientAuth])
                KeyUsage(digitalSignature: true)
            },
            issuerPrivateKey: swiftKey
        )

        // 2. Serialize cert to DER
        var s = DER.Serializer()
        try cert.serialize(into: &s)
        let certDER = Data(s.serializedBytes)
        try certDER.write(to: certDERURL)

        // 3. Write temp PEM files and bundle as PKCS12 via openssl
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("tunnel-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }

        let certPEM = tmp.appendingPathComponent("cert.pem")
        let keyPEM  = tmp.appendingPathComponent("key.pem")

        let b64Cert = certDER.base64EncodedString(options: [.lineLength64Characters, .endLineWithLineFeed])
        try "-----BEGIN CERTIFICATE-----\n\(b64Cert)\n-----END CERTIFICATE-----\n"
            .write(to: certPEM, atomically: true, encoding: .utf8)

        let keyDER = privateKey.derRepresentation
        let b64Key = keyDER.base64EncodedString(options: [.lineLength64Characters, .endLineWithLineFeed])
        try "-----BEGIN PRIVATE KEY-----\n\(b64Key)\n-----END PRIVATE KEY-----\n"
            .write(to: keyPEM, atomically: true, encoding: .utf8)

        try runOpenSSL([
            "pkcs12", "-export",
            "-in",    certPEM.path,
            "-inkey", keyPEM.path,
            "-out",   p12URL.path,
            "-passout", "pass:\(p12Password)"
        ])

        let secIdentity = try importP12(at: p12URL)
        let fp = fingerprintOf(secIdentity)
        return (secIdentity, fp)
    }

    // MARK: - P12 import

    private static let p12Password = "tunnel-p12-internal-v1"

    private static func importP12(at url: URL) throws -> SecIdentity {
        let data = try Data(contentsOf: url)

        var accessRef: SecAccess?
        SecAccessCreate("Tunnel TLS Identity" as CFString, [] as CFArray, &accessRef)

        var options: [String: Any] = [kSecImportExportPassphrase as String: p12Password]
        if let access = accessRef {
            options[kSecImportExportAccess as String] = access
        }

        var items: CFArray?
        let status = SecPKCS12Import(data as CFData, options as CFDictionary, &items)
        guard status == errSecSuccess,
              let arr = items as? [[String: Any]],
              let first = arr.first,
              let ref = first[kSecImportItemIdentity as String]
        else { throw TLSError.keychainError(status) }
        return ref as! SecIdentity
    }

    // MARK: - Fingerprint helpers

    private static func fingerprintOf(_ identity: SecIdentity) -> String {
        var certRef: SecCertificate?
        SecIdentityCopyCertificate(identity, &certRef)
        guard let cert = certRef else { return "unknown" }
        return SHA256.hash(data: SecCertificateCopyData(cert) as Data).hexString
    }

    // MARK: - openssl helper

    private static func runOpenSSL(_ args: [String]) throws {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/openssl")
        p.arguments = args
        p.standardOutput = FileHandle.nullDevice
        p.standardError  = FileHandle.nullDevice
        try p.run()
        p.waitUntilExit()
        guard p.terminationStatus == 0 else {
            throw TLSError.opensslFailed(p.terminationStatus)
        }
    }
}

// MARK: - Errors

enum TLSError: Error, LocalizedError {
    case invalidCertificate
    case keychainError(OSStatus)
    case identityNotFound(OSStatus)
    case opensslFailed(Int32)

    var errorDescription: String? {
        switch self {
        case .invalidCertificate:       return "Could not create certificate from DER data"
        case .keychainError(let s):     return "Keychain error: \(s)"
        case .identityNotFound(let s):  return "Identity not found: \(s)"
        case .opensslFailed(let s):     return "openssl exited with code \(s)"
        }
    }
}

// MARK: - Helpers

private extension SHA256Digest {
    var hexString: String { map { String(format: "%02x", $0) }.joined() }
}
