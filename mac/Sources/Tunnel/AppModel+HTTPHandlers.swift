import Foundation
import Network
import AppKit

// MARK: - HTTP connection handler

extension AppModel {

    func handleHTTPConnection(_ conn: NWConnection) {
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
            filesRemaining: fileCount, totalBytes: totalBytes,
            senderFingerprint: req.info.fingerprint
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
                let elapsed = max(session.startTime.timeIntervalSinceNow * -1, 0.1)
                let bps = UInt64(Double(totalReceived) / elapsed)
                let eta: UInt64? = bps > 0 && totalReceived < session.totalBytes
                    ? UInt64((session.totalBytes - totalReceived) / bps) : nil
                peerProgress[session.senderFingerprint] = .transferring(
                    bytesDone: totalReceived, totalBytes: session.totalBytes,
                    bytesPerSec: bps, etaSecs: eta
                )
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
        let fp = s.senderFingerprint
        s.filesRemaining -= 1
        if s.filesRemaining <= 0 {
            sessions.removeValue(forKey: sessionId)
            peerProgress[fp] = .complete
            NSWorkspace.shared.selectFile(destURL.path,
                inFileViewerRootedAtPath: destURL.deletingLastPathComponent().path)
            Task {
                try? await Task.sleep(for: .seconds(1.2))
                self.peerProgress.removeValue(forKey: fp)
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

// MARK: - Upload session state

struct UploadSession {
    var files: [String: FileMetadata]
    var tokens: [String: String]       // fileId → token
    var downloadDir: URL
    var filesRemaining: Int
    var totalBytes: UInt64
    var senderFingerprint: String
    var startTime: Date = Date()
}
