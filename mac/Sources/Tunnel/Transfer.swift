import Foundation
import CryptoKit

let maxFileSize: UInt64 = 10 * 1024 * 1024 * 1024  // 10 GiB

// MARK: - Send

/// Send one or more files to a LocalSend v2 peer.
///
/// Flow:
///   1. POST /api/localsend/v2/prepare-upload  — offer file list, wait for accept.
///   2. POST /api/localsend/v2/upload          — upload each file's bytes.
func sendFiles(
    to peer: Peer,
    fileURLs: [URL],
    senderAlias: String,
    senderFingerprint: String,
    tlsManager: TLSManager,
    progress: @escaping (UInt64, UInt64) -> Void  // (bytesDone, totalBytes)
) async throws {
    let baseURL = "https://\(peer.host):\(peer.port)"

    let delegate = TofuSessionDelegate(tlsManager: tlsManager, peerFingerprint: peer.id)
    let session = URLSession(configuration: .ephemeral, delegate: delegate, delegateQueue: nil)
    defer { session.invalidateAndCancel() }

    // Build per-file metadata.
    struct FileEntry {
        let fileId: String
        let data: Data
        let fileName: String
        let size: UInt64
        let fileType: String
        let sha256: String
    }

    var entries: [FileEntry] = []
    for url in fileURLs {
        let attrs = try FileManager.default.attributesOfItem(atPath: url.path)
        let size = (attrs[.size] as? UInt64) ?? 0
        guard size <= maxFileSize else { throw TransferError.fileTooLarge }
        let data = try Data(contentsOf: url)
        let sha256 = SHA256.hash(data: data).hexString
        entries.append(FileEntry(
            fileId: UUID().uuidString,
            data: data,
            fileName: url.lastPathComponent,
            size: size,
            fileType: mimeType(for: url.pathExtension),
            sha256: sha256
        ))
    }

    let totalBytes = entries.reduce(UInt64(0)) { $0 + $1.size }

    // 1. prepare-upload
    let filesMetadata = Dictionary(uniqueKeysWithValues: entries.map { e in
        (e.fileId, FileMetadata(id: e.fileId, fileName: e.fileName, size: e.size,
                                fileType: e.fileType, sha256: e.sha256))
    })

    let prepareReq = PrepareUploadRequest(
        info: DeviceInfo(
            alias: senderAlias,
            version: "2.0",
            deviceModel: "Mac",
            deviceType: "desktop",
            fingerprint: senderFingerprint,
            port: localsendPort,
            protocolScheme: "https",
            download: false
        ),
        files: filesMetadata
    )

    var req = URLRequest(url: URL(string: "\(baseURL)/api/localsend/v2/prepare-upload")!)
    req.httpMethod = "POST"
    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
    req.httpBody = try JSONEncoder().encode(prepareReq)

    let (prepareData, prepareResponse) = try await session.data(for: req)
    guard let http = prepareResponse as? HTTPURLResponse else { throw TransferError.unexpectedResponse }
    if http.statusCode == 403 { throw TransferError.denied }
    guard http.statusCode == 200 else { throw TransferError.unexpectedResponse }

    let prepareResp = try JSONDecoder().decode(PrepareUploadResponse.self, from: prepareData)

    // 2. Upload each file.
    var bytesDone = UInt64(0)
    for entry in entries {
        guard let token = prepareResp.files[entry.fileId] else { throw TransferError.unexpectedResponse }

        var components = URLComponents(string: "\(baseURL)/api/localsend/v2/upload")!
        components.queryItems = [
            URLQueryItem(name: "sessionId", value: prepareResp.sessionId),
            URLQueryItem(name: "fileId",    value: entry.fileId),
            URLQueryItem(name: "token",     value: token),
        ]

        var uploadReq = URLRequest(url: components.url!)
        uploadReq.httpMethod = "POST"
        uploadReq.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        uploadReq.setValue(String(entry.size), forHTTPHeaderField: "Content-Length")

        // Report progress during this file's upload via the delegate.
        let bytesDoneBeforeFile = bytesDone
        delegate.progressHandler = { sentThisFile in
            let overall = bytesDoneBeforeFile + UInt64(sentThisFile)
            progress(overall, totalBytes)
        }

        let (_, uploadResp) = try await session.upload(for: uploadReq, from: entry.data)
        delegate.progressHandler = nil
        guard let uploadHttp = uploadResp as? HTTPURLResponse, uploadHttp.statusCode == 200 else {
            throw TransferError.uploadFailed
        }

        bytesDone += entry.size
    }
    progress(totalBytes, totalBytes)
}

// MARK: - TOFU URLSession delegate

/// Verifies the receiver's self-signed cert using TOFU keyed by SHA-256 fingerprint.
/// First contact is trusted; subsequent contacts must match the stored fingerprint.
final class TofuSessionDelegate: NSObject, URLSessionDelegate, URLSessionTaskDelegate {
    private let tlsManager: TLSManager
    private let peerFingerprint: String  // peer's announced identity from UDP discovery
    /// Called during upload with the number of bytes sent for the current file.
    var progressHandler: ((Int64) -> Void)?

    init(tlsManager: TLSManager, peerFingerprint: String) {
        self.tlsManager = tlsManager
        self.peerFingerprint = peerFingerprint
    }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        didSendBodyData bytesSent: Int64,
        totalBytesSent: Int64,
        totalBytesExpectedToSend: Int64
    ) {
        progressHandler?(totalBytesSent)
    }

    func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        guard let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
              let cert = chain.first else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        let actualCertFingerprint = SHA256.hash(data: SecCertificateCopyData(cert) as Data).hexString
        let expectedPeerFp = peerFingerprint
        Task {
            let allowed = await tlsManager.verifyPeerCert(
                peerFingerprint: expectedPeerFp,
                actualCertFingerprint: actualCertFingerprint
            )
            completionHandler(
                allowed ? .useCredential : .cancelAuthenticationChallenge,
                allowed ? URLCredential(trust: trust) : nil
            )
        }
    }
}

// MARK: - Errors

enum TransferError: Error, LocalizedError {
    case denied
    case fileTooLarge
    case unexpectedResponse
    case uploadFailed
    case connectionFailed(Error)

    var errorDescription: String? {
        switch self {
        case .denied:              return "Peer denied the transfer"
        case .fileTooLarge:        return "File exceeds the 10 GiB limit"
        case .unexpectedResponse:  return "Unexpected response from peer"
        case .uploadFailed:        return "File upload failed"
        case .connectionFailed(let e): return "Connection failed: \(e.localizedDescription)"
        }
    }
}

// MARK: - Helpers

func sanitizeFilename(_ name: String) -> String {
    let sanitized = name.unicodeScalars
        .map { "/\\:*?\"<>|".unicodeScalars.contains($0) ? Character("_") : Character($0) }
        .map(String.init)
        .joined()
    return (sanitized == ".." || sanitized == ".") ? "file" : sanitized
}

private func mimeType(for ext: String) -> String {
    switch ext.lowercased() {
    case "jpg", "jpeg": return "image/jpeg"
    case "png":   return "image/png"
    case "gif":   return "image/gif"
    case "webp":  return "image/webp"
    case "pdf":   return "application/pdf"
    case "zip":   return "application/zip"
    case "mp4":   return "video/mp4"
    case "mp3":   return "audio/mpeg"
    case "txt":   return "text/plain"
    default:      return "application/octet-stream"
    }
}

private extension SHA256Digest {
    var hexString: String { map { String(format: "%02x", $0) }.joined() }
}
