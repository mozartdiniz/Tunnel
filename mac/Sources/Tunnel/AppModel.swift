import Foundation
import Network
import SwiftUI
import CryptoKit

// MARK: - AppModel

@MainActor
final class AppModel: ObservableObject {

    @Published var peers: [Peer] = []
    @Published var statusMessage: String = "Starting…"
    @Published var transferProgress: Double = 0
    @Published var activeTransfer: String?
    @Published var pendingRequest: PendingRequest?
    @Published var deviceName: String = ""
    @Published var downloadDir: URL = FileManager.default
        .urls(for: .downloadsDirectory, in: .userDomainMask)[0]

    private var config: Config
    private var tlsManager: TLSManager?
    private var discovery: Discovery?
    private var httpListener: NWListener?
    private let serverQueue = DispatchQueue(label: "dev.tunnel.server", qos: .userInitiated)

    // Pending incoming transfers: sessionId → continuation resolving to accept/deny.
    private var pendingDecisions: [String: CheckedContinuation<Bool, Never>] = [:]
    // Active upload sessions keyed by sessionId.
    private var sessions: [String: UploadSession] = [:]

    init(config: Config = .load()) {
        self.config = config
        self.deviceName = config.deviceName
        self.downloadDir = config.downloadDir
    }

    // MARK: - Startup

    func start() async {
        statusMessage = "Initializing…"
        do {
            let tls = try await TLSManager.loadOrCreate(config: config)
            self.tlsManager = tls
            let fingerprint = await tls.localFingerprint

            // Start HTTPS server (receive side).
            try await startHTTPSServer(tls: tls)

            // Start UDP multicast discovery.
            let disc = Discovery(fingerprint: fingerprint)
            disc.onPeerFound = { [weak self] peer in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if !self.peers.contains(where: { $0.id == peer.id }) {
                        self.peers.append(peer)
                    }
                }
            }
            disc.onPeerLost = { [weak self] fp in
                Task { @MainActor [weak self] in
                    self?.peers.removeAll { $0.id == fp }
                }
            }
            self.discovery = disc
            disc.advertise(alias: config.deviceName, port: localsendPort)
            disc.startBrowsing(alias: config.deviceName, port: localsendPort)

            statusMessage = "Ready — \(config.deviceName)"
        } catch {
            statusMessage = "Error: \(error.localizedDescription)"
        }
    }

    // MARK: - HTTPS server

    private func startHTTPSServer(tls: TLSManager) async throws {
        let params = await tls.listenerParameters()
        let listener = try NWListener(using: params, on: NWEndpoint.Port(integerLiteral: localsendPort))
        self.httpListener = listener

        listener.newConnectionHandler = { [weak self] conn in
            Task { [weak self] in await self?.handleHTTPConnection(conn) }
        }
        listener.stateUpdateHandler = { state in
            switch state {
            case .ready:           print("[Server] HTTPS listener ready on :\(localsendPort)")
            case .failed(let err): print("[Server] Listener failed: \(err)")
            default: break
            }
        }
        listener.start(queue: serverQueue)
    }

    // MARK: - Send

    func sendFile(to peer: Peer, fileURL: URL) {
        guard let tlsManager else { return }
        activeTransfer = "Sending \(fileURL.lastPathComponent)…"
        transferProgress = 0

        Task {
            let alias = config.deviceName
            let fp: String = await tlsManager.localFingerprint
            do {
                try await sendFiles(
                    to: peer,
                    fileURLs: [fileURL],
                    senderAlias: alias,
                    senderFingerprint: fp,
                    tlsManager: tlsManager,
                    progress: { [weak self] pct in
                        Task { @MainActor [weak self] in self?.transferProgress = pct }
                    }
                )
                activeTransfer = "Sent \(fileURL.lastPathComponent) ✓"
                transferProgress = 1
                try? await Task.sleep(for: .seconds(3))
                activeTransfer = nil
            } catch TransferError.denied {
                activeTransfer = "Transfer declined"
                try? await Task.sleep(for: .seconds(3))
                activeTransfer = nil
            } catch {
                activeTransfer = "Send failed: \(error.localizedDescription)"
                try? await Task.sleep(for: .seconds(5))
                activeTransfer = nil
            }
        }
    }

    // MARK: - User decisions

    func acceptTransfer(_ id: String) {
        pendingDecisions.removeValue(forKey: id)?.resume(returning: true)
    }

    func denyTransfer(_ id: String) {
        pendingDecisions.removeValue(forKey: id)?.resume(returning: false)
    }

    // MARK: - Settings

    func updateDeviceName(_ name: String) {
        config.deviceName = name
        config.save()
        deviceName = name
        discovery?.advertise(alias: name, port: localsendPort)
    }

    func updateDownloadDir(_ url: URL) {
        config.downloadDir = url
        config.save()
        downloadDir = url
    }
}

// MARK: - HTTP connection handler

extension AppModel {

    fileprivate func handleHTTPConnection(_ conn: NWConnection) {
        conn.start(queue: serverQueue)
        Task { [weak self] in
            guard let self else { return }
            do {
                let (method, rawPath, headers, bodyStart) = try await readHTTPHeaders(conn)
                let contentLength = headers["content-length"].flatMap(Int.init) ?? 0

                let pathOnly = rawPath.components(separatedBy: "?").first ?? rawPath
                let queryStr  = rawPath.components(separatedBy: "?").dropFirst().first ?? ""
                let params    = parseQueryString(queryStr)

                switch (method, pathOnly) {
                case ("GET", "/api/localsend/v2/info"):
                    try await handleInfo(conn)

                case ("POST", "/api/localsend/v2/prepare-upload"):
                    let body = try await readBody(conn, alreadyRead: bodyStart, remaining: contentLength - bodyStart.count)
                    try await handlePrepareUpload(conn, body: body)

                case ("POST", "/api/localsend/v2/upload"):
                    try await handleUpload(conn, bodyStart: bodyStart, contentLength: contentLength, params: params)

                case ("POST", "/api/localsend/v2/cancel"):
                    try await handleCancel(conn, params: params)

                default:
                    try await sendHTTPResponse(conn, status: 404, body: Data())
                }
            } catch {
                conn.cancel()
            }
        }
    }

    // GET /api/localsend/v2/info
    private func handleInfo(_ conn: NWConnection) async throws {
        let alias = config.deviceName
        let fp    = await tlsManager?.localFingerprint ?? "unknown"
        let info  = DeviceInfo(
            alias: alias, version: "2.0", deviceModel: "Mac", deviceType: "desktop",
            fingerprint: fp, port: localsendPort, protocolScheme: "https", download: false
        )
        let body = try JSONEncoder().encode(info)
        try await sendHTTPResponse(conn, status: 200,
                                   headers: ["Content-Type": "application/json"], body: body)
    }

    // POST /api/localsend/v2/prepare-upload
    private func handlePrepareUpload(_ conn: NWConnection, body: Data) async throws {
        guard let req = try? JSONDecoder().decode(PrepareUploadRequest.self, from: body) else {
            try await sendHTTPResponse(conn, status: 400, body: Data())
            return
        }

        let sessionId  = UUID().uuidString
        let fileCount  = req.files.count
        let totalBytes = req.files.values.reduce(UInt64(0)) { $0 + $1.size }
        let firstName  = req.files.values.first?.fileName ?? "file"

        let tokens = Dictionary(uniqueKeysWithValues: req.files.keys.map { ($0, UUID().uuidString) })

        // Register decision channel before notifying the UI.
        // All handler methods run on @MainActor, so we can access state directly.
        let accepted: Bool = await withCheckedContinuation { cont in
            self.pendingDecisions[sessionId] = cont
            self.pendingRequest = PendingRequest(
                transferId: sessionId,
                senderName: req.info.alias,
                fileName: fileCount > 1 ? "\(firstName) + \(fileCount - 1) more" : firstName,
                sizeBytes: totalBytes
            )
            // Auto-deny after 60 seconds if the user doesn't respond.
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(60))
                guard let self else { return }
                if let cont = self.pendingDecisions.removeValue(forKey: sessionId) {
                    self.pendingRequest = nil
                    cont.resume(returning: false)
                }
            }
        }

        pendingRequest = nil

        guard accepted else {
            try await sendHTTPResponse(conn, status: 403, body: Data())
            return
        }

        let dlDir = config.downloadDir
        let session = UploadSession(
            files: req.files, tokens: tokens, downloadDir: dlDir,
            filesRemaining: fileCount, totalBytes: totalBytes
        )
        sessions[sessionId] = session

        let resp = PrepareUploadResponse(sessionId: sessionId, files: tokens)
        let body = try JSONEncoder().encode(resp)
        try await sendHTTPResponse(conn, status: 200,
                                   headers: ["Content-Type": "application/json"], body: body)
    }

    // POST /api/localsend/v2/upload?sessionId=&fileId=&token=
    private func handleUpload(
        _ conn: NWConnection, bodyStart: Data, contentLength: Int, params: [String: String]
    ) async throws {
        guard let sessionId = params["sessionId"],
              let fileId    = params["fileId"],
              let token     = params["token"] else {
            try await sendHTTPResponse(conn, status: 400, body: Data())
            return
        }

        guard let session = sessions[sessionId],
              session.tokens[fileId] == token,
              let fileMeta = session.files[fileId] else {
            try await sendHTTPResponse(conn, status: 403, body: Data())
            return
        }

        // Reject files that exceed the 10 GiB limit.
        guard fileMeta.size <= maxFileSize else {
            try await sendHTTPResponse(conn, status: 413, body: Data())
            return
        }

        // Reject if insufficient disk space.
        if let attrs = try? FileManager.default.attributesOfFileSystem(
            forPath: session.downloadDir.path),
           let free = attrs[.systemFreeSize] as? UInt64,
           fileMeta.size > free
        {
            try await sendHTTPResponse(conn, status: 507, body: Data())
            return
        }

        let safeName = sanitizeFilename(fileMeta.fileName)
        let destURL  = session.downloadDir.appendingPathComponent(safeName)
        let tmpURL   = session.downloadDir.appendingPathComponent(".\(sessionId).\(fileId).tmp")

        FileManager.default.createFile(atPath: tmpURL.path, contents: nil)
        let handle = try FileHandle(forWritingTo: tmpURL)

        do {
            try handle.write(contentsOf: bodyStart)
            var received = bodyStart.count
            while received < contentLength {
                let toRead = min(65536, contentLength - received)
                let chunk  = try await receiveChunk(conn, maxLength: toRead)
                try handle.write(contentsOf: chunk)
                received += chunk.count
                let totalReceived = UInt64(received)
                transferProgress = session.totalBytes > 0
                    ? Double(totalReceived) / Double(session.totalBytes) : 0
                activeTransfer = "Receiving \(fileMeta.fileName)…"
            }
            try handle.close()
        } catch {
            try? handle.close()
            try? FileManager.default.removeItem(at: tmpURL)
            try await sendHTTPResponse(conn, status: 500, body: Data())
            return
        }

        // Atomic rename.
        if FileManager.default.fileExists(atPath: destURL.path) {
            try? FileManager.default.removeItem(at: destURL)
        }
        try FileManager.default.moveItem(at: tmpURL, to: destURL)
        try await sendHTTPResponse(conn, status: 200, body: Data())

        // Decrement file counter; notify UI when last file arrives.
        guard var s = sessions[sessionId] else { return }
        s.filesRemaining -= 1
        if s.filesRemaining <= 0 {
            sessions.removeValue(forKey: sessionId)
            activeTransfer = "Received ✓"
            transferProgress = 1
            NSWorkspace.shared.selectFile(destURL.path,
                inFileViewerRootedAtPath: destURL.deletingLastPathComponent().path)
            Task {
                try? await Task.sleep(for: .seconds(3))
                self.activeTransfer = nil
            }
        } else {
            sessions[sessionId] = s
        }
    }

    // POST /api/localsend/v2/cancel?sessionId=
    private func handleCancel(_ conn: NWConnection, params: [String: String]) async throws {
        if let sessionId = params["sessionId"] {
            sessions.removeValue(forKey: sessionId)
            pendingDecisions.removeValue(forKey: sessionId)?.resume(returning: false)
        }
        try await sendHTTPResponse(conn, status: 200, body: Data())
    }
}

// MARK: - NWConnection async helpers

private func receiveChunk(_ conn: NWConnection, maxLength: Int) async throws -> Data {
    try await withCheckedThrowingContinuation { cont in
        conn.receive(minimumIncompleteLength: 1, maximumLength: maxLength) { data, _, isComplete, error in
            if let error { cont.resume(throwing: error) }
            else if let data, !data.isEmpty { cont.resume(returning: data) }
            else if isComplete { cont.resume(throwing: NWError.posix(.ECONNRESET)) }
            else { cont.resume(throwing: NWError.posix(.EAGAIN)) }
        }
    }
}

private func readHTTPHeaders(_ conn: NWConnection) async throws
    -> (method: String, path: String, headers: [String: String], bodyStart: Data)
{
    var buffer = Data()
    let delimiter = Data("\r\n\r\n".utf8)

    while buffer.count < 65536 {
        let chunk = try await receiveChunk(conn, maxLength: 4096)
        buffer.append(chunk)
        if let range = buffer.range(of: delimiter) {
            let headerBytes = buffer[..<range.lowerBound]
            let bodyStart   = Data(buffer[range.upperBound...])
            let headerStr   = String(decoding: headerBytes, as: UTF8.self)
            let lines       = headerStr.components(separatedBy: "\r\n")
            guard let requestLine = lines.first else { throw HTTPError.badRequest }
            let parts = requestLine.split(separator: " ", maxSplits: 2)
            guard parts.count >= 2 else { throw HTTPError.badRequest }
            var headers: [String: String] = [:]
            for line in lines.dropFirst() {
                guard let colonIdx = line.firstIndex(of: ":") else { continue }
                let key = String(line[..<colonIdx]).trimmingCharacters(in: .whitespaces).lowercased()
                let val = String(line[line.index(after: colonIdx)...]).trimmingCharacters(in: .whitespaces)
                headers[key] = val
            }
            return (String(parts[0]), String(parts[1]), headers, bodyStart)
        }
    }
    throw HTTPError.requestTooLarge
}

private func readBody(_ conn: NWConnection, alreadyRead: Data, remaining: Int) async throws -> Data {
    var body = alreadyRead
    var left = remaining
    while left > 0 {
        let chunk = try await receiveChunk(conn, maxLength: min(65536, left))
        body.append(chunk)
        left -= chunk.count
    }
    return body
}

private func sendHTTPResponse(
    _ conn: NWConnection, status: Int,
    headers: [String: String] = [:], body: Data
) async throws {
    var lines = ["HTTP/1.1 \(status) \(httpStatusText(status))"]
    lines.append("Content-Length: \(body.count)")
    lines.append("Connection: close")
    for (k, v) in headers { lines.append("\(k): \(v)") }
    lines.append("")
    lines.append("")
    var response = Data(lines.joined(separator: "\r\n").utf8)
    response.append(body)
    try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
        conn.send(content: response, completion: .contentProcessed { error in
            if let e = error { cont.resume(throwing: e) } else { cont.resume() }
        })
    }
    conn.cancel()
}

private func parseQueryString(_ query: String) -> [String: String] {
    var result: [String: String] = [:]
    for item in query.split(separator: "&") {
        let kv = item.split(separator: "=", maxSplits: 1)
        guard kv.count == 2 else { continue }
        let k = String(kv[0]).removingPercentEncoding ?? String(kv[0])
        let v = String(kv[1]).removingPercentEncoding ?? String(kv[1])
        result[k] = v
    }
    return result
}

private func httpStatusText(_ code: Int) -> String {
    switch code {
    case 200: return "OK"
    case 400: return "Bad Request"
    case 403: return "Forbidden"
    case 404: return "Not Found"
    case 413: return "Content Too Large"
    case 500: return "Internal Server Error"
    case 507: return "Insufficient Storage"
    default:  return "Unknown"
    }
}

private enum HTTPError: Error {
    case badRequest
    case requestTooLarge
}

// MARK: - Upload session state

private struct UploadSession {
    var files: [String: FileMetadata]
    var tokens: [String: String]       // fileId → token
    var downloadDir: URL
    var filesRemaining: Int
    var totalBytes: UInt64
}

// MARK: - PendingRequest

struct PendingRequest: Identifiable {
    let id = UUID()
    let transferId: String
    let senderName: String
    let fileName: String
    let sizeBytes: UInt64

    var formattedSize: String {
        ByteCountFormatter.string(fromByteCount: Int64(sizeBytes), countStyle: .file)
    }
}
