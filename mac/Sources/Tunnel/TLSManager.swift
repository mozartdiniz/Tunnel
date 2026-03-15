import Foundation
import Network
import Security
import CryptoKit
import X509
import SwiftASN1

// MARK: - TLSManager

/// Manages the local TLS identity (self-signed cert) and TOFU peer verification.
///
/// Identity strategy: generate cert+key with swift-certificates, bundle as PKCS12 via
/// `/usr/bin/openssl`, then import with `SecPKCS12Import` which works in unsigned apps
/// (it uses the login keychain path, not the data-protection keychain that requires entitlements).
actor TLSManager {

    private let identity: SecIdentity
    private var knownPeers: [String: String]  // peer-key -> SHA-256 fingerprint hex
    private let peersFileURL: URL

    // MARK: - Bootstrap

    static func loadOrCreate(config: Config) async throws -> TLSManager {
        let dir = Config.dataDir()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        let p12URL    = dir.appendingPathComponent("identity.p12")
        let certDERURL = dir.appendingPathComponent("cert.der")
        let peersURL  = dir.appendingPathComponent("known_peers.json")

        let identity: SecIdentity
        if FileManager.default.fileExists(atPath: p12URL.path) {
            identity = try importP12(at: p12URL)
        } else {
            identity = try generateAndSave(
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

        return TLSManager(identity: identity, knownPeers: knownPeers, peersFileURL: peersURL)
    }

    private init(identity: SecIdentity, knownPeers: [String: String], peersFileURL: URL) {
        self.identity = identity
        self.knownPeers = knownPeers
        self.peersFileURL = peersFileURL
    }

    // MARK: - NWParameters

    func listenerParameters() -> NWParameters {
        makeParameters(isServer: true)
    }

    func connectionParameters() -> NWParameters {
        makeParameters(isServer: false)
    }

    private func makeParameters(isServer: Bool) -> NWParameters {
        let tlsOptions = NWProtocolTLS.Options()
        let capturedIdentity = identity

        sec_protocol_options_set_local_identity(
            tlsOptions.securityProtocolOptions,
            sec_identity_create(capturedIdentity)!
        )

        // Disable hostname validation — we do TOFU fingerprint checks instead
        sec_protocol_options_set_peer_authentication_required(
            tlsOptions.securityProtocolOptions, false
        )

        if !isServer {
            sec_protocol_options_set_verify_block(
                tlsOptions.securityProtocolOptions,
                { [weak self] metadata, trust, complete in
                    guard let self else { complete(false); return }
                    Task { complete(await self.verifyPeer(metadata: metadata, trust: trust)) }
                },
                DispatchQueue.global()
            )
        }

        return NWParameters(tls: tlsOptions, tcp: NWProtocolTCP.Options())
    }

    // MARK: - TOFU

    private func verifyPeer(metadata: sec_protocol_metadata_t, trust: sec_trust_t) async -> Bool {
        var chain: [SecCertificate] = []
        sec_protocol_metadata_access_peer_certificate_chain(metadata) { secCert in
            chain.append(sec_certificate_copy_ref(secCert).takeRetainedValue())
        }
        guard let leaf = chain.first else { return false }

        let fingerprint = SHA256.hash(data: SecCertificateCopyData(leaf) as Data).hexString
        let key = peerKey(from: metadata)

        if let stored = knownPeers[key] {
            return stored == fingerprint      // mismatch = TOFU violation, reject
        } else {
            knownPeers[key] = fingerprint
            persistPeers()
            return true
        }
    }

    private func peerKey(from metadata: sec_protocol_metadata_t) -> String {
        if let sn = sec_protocol_metadata_get_server_name(metadata) {
            return String(cString: sn)
        }
        return "unknown"
    }

    private func persistPeers() {
        guard let data = try? JSONEncoder().encode(knownPeers) else { return }
        try? data.write(to: peersFileURL)
    }

    func localFingerprint() -> String {
        var certRef: SecCertificate?
        SecIdentityCopyCertificate(identity, &certRef)
        guard let cert = certRef else { return "unknown" }
        return SHA256.hash(data: SecCertificateCopyData(cert) as Data).hexString
    }

    // MARK: - Generation

    private static func generateAndSave(
        deviceName: String,
        p12URL: URL,
        certDERURL: URL
    ) throws -> SecIdentity {
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
            },
            issuerPrivateKey: swiftKey
        )

        // 2. Serialize cert to DER
        var s = DER.Serializer()
        try cert.serialize(into: &s)
        let certDER = Data(s.serializedBytes)
        try certDER.write(to: certDERURL)

        // 3. Write temp PEM files for openssl pkcs12 bundling
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

        // 4. Bundle as PKCS12 using the system openssl (LibreSSL, always present on macOS)
        try runOpenSSL([
            "pkcs12", "-export",
            "-in",  certPEM.path,
            "-inkey", keyPEM.path,
            "-out", p12URL.path,
            "-passout", "pass:\(p12Password)"
        ])

        return try importP12(at: p12URL)
    }

    // MARK: - P12 import

    private static let p12Password = "tunnel-p12-internal-v1"

    /// Import a PKCS12 file and return the SecIdentity.
    /// SecPKCS12Import targets the login keychain, which doesn't require code-signing
    /// entitlements — unlike SecItemAdd for kSecClassKey which needs data-protection rights.
    private static func importP12(at url: URL) throws -> SecIdentity {
        let data = try Data(contentsOf: url)
        var items: CFArray?
        let status = SecPKCS12Import(
            data as CFData,
            [kSecImportExportPassphrase as String: p12Password] as CFDictionary,
            &items
        )
        guard status == errSecSuccess,
              let arr = items as? [[String: Any]],
              let first = arr.first,
              let ref = first[kSecImportItemIdentity as String]
        else { throw TLSError.keychainError(status) }
        return ref as! SecIdentity
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
