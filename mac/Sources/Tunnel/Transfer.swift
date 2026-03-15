import Foundation
import Network
import CryptoKit

private let chunkSize = 64 * 1024  // 64 KiB

// MARK: - Outgoing

/// Send a file to a peer. Returns when the transfer is complete.
func sendFile(
    to endpoint: NWEndpoint,
    fileURL: URL,
    senderName: String,
    parameters: NWParameters,
    progress: @escaping (Double) -> Void
) async throws {
    let conn = NWConnection(to: endpoint, using: parameters)
    try await connect(conn)
    defer { conn.cancel() }

    // Compute size and checksum upfront
    let fileData = try Data(contentsOf: fileURL)
    let sizeBytes = UInt64(fileData.count)
    let checksum = SHA256.hash(data: fileData).hexString
    let fileName = fileURL.lastPathComponent
    let transferId = UUID().uuidString

    // Send ASK
    let ask = TunnelMessage.ask(
        version: protocolVersion,
        transferId: transferId,
        senderName: senderName,
        fileName: fileName,
        sizeBytes: sizeBytes
    )
    try await send(conn, data: encodeMessage(ask))

    // Wait for RESPONSE
    let responseData = try await receiveMessage(conn)
    guard case .response(_, let status) = responseData, status == .accepted else {
        if case .response(_, let s) = responseData, s == .denied {
            throw TransferError.denied
        }
        throw TransferError.unexpectedMessage
    }

    // Stream file bytes in chunks
    var sent = 0
    while sent < fileData.count {
        let end = min(sent + chunkSize, fileData.count)
        let chunk = fileData[sent..<end]
        try await send(conn, data: Data(chunk))
        sent = end
        progress(Double(sent) / Double(fileData.count))
    }

    // Send DONE with checksum
    let done = TunnelMessage.done(checksumSha256: checksum)
    try await send(conn, data: encodeMessage(done))

    // Wait for ACK
    let ackData = try await receiveMessage(conn)
    guard case .response(_, let ackStatus) = ackData else {
        throw TransferError.unexpectedMessage
    }
    if ackStatus == .checksumFail {
        throw TransferError.checksumMismatch
    }
}

// MARK: - Incoming

struct IncomingTransferRequest {
    let transferId: String
    let senderName: String
    let fileName: String
    let sizeBytes: UInt64
}

/// Handle an incoming connection: read the ASK, return the request details.
/// After user decision, call `completeReceive`.
func readIncomingAsk(conn: NWConnection) async throws -> IncomingTransferRequest {
    try await connect(conn)
    let msg = try await receiveMessage(conn)
    guard case .ask(_, let transferId, let senderName, let fileName, let sizeBytes) = msg else {
        conn.cancel()
        throw TransferError.unexpectedMessage
    }
    return IncomingTransferRequest(
        transferId: transferId,
        senderName: senderName,
        fileName: fileName,
        sizeBytes: sizeBytes
    )
}

/// After user accepts/denies, complete the receive side of the transfer.
func completeReceive(
    conn: NWConnection,
    request: IncomingTransferRequest,
    accepted: Bool,
    downloadDir: URL,
    progress: @escaping (Double) -> Void
) async throws -> URL? {
    defer { conn.cancel() }

    // Send RESPONSE
    let status: ResponseStatus = accepted ? .accepted : .denied
    let response = TunnelMessage.response(version: protocolVersion, status: status)
    try await send(conn, data: encodeMessage(response))

    guard accepted else { return nil }

    // Receive file bytes
    let safeName = sanitizeFilename(request.fileName)
    let destURL = downloadDir.appendingPathComponent(safeName)

    var received = Data()
    received.reserveCapacity(Int(request.sizeBytes))
    var hasher = SHA256()

    while received.count < Int(request.sizeBytes) {
        let remaining = Int(request.sizeBytes) - received.count
        let toRead = min(chunkSize, remaining)
        let chunk = try await receiveExact(conn, count: toRead)
        received.append(chunk)
        hasher.update(data: chunk)
        progress(Double(received.count) / Double(request.sizeBytes))
    }

    // Read DONE message
    let doneMsg = try await receiveMessage(conn)
    guard case .done(let expectedChecksum) = doneMsg else {
        throw TransferError.unexpectedMessage
    }

    let actualChecksum = hasher.finalize().hexString
    let checksumOk = actualChecksum == expectedChecksum

    // Send checksum ACK
    let ackStatus: ResponseStatus = checksumOk ? .checksumOk : .checksumFail
    let ack = TunnelMessage.response(version: protocolVersion, status: ackStatus)
    try await send(conn, data: encodeMessage(ack))

    if checksumOk {
        try received.write(to: destURL)
        return destURL
    } else {
        throw TransferError.checksumMismatch
    }
}

// MARK: - Errors

enum TransferError: Error, LocalizedError {
    case denied
    case checksumMismatch
    case unexpectedMessage
    case connectionFailed(Error)
    case timeout

    var errorDescription: String? {
        switch self {
        case .denied:            return "Peer denied the transfer"
        case .checksumMismatch:  return "File checksum mismatch — transfer may be corrupted"
        case .unexpectedMessage: return "Unexpected protocol message"
        case .connectionFailed(let e): return "Connection failed: \(e.localizedDescription)"
        case .timeout:           return "Transfer timed out"
        }
    }
}

// MARK: - NWConnection async helpers

private func connect(_ conn: NWConnection) async throws {
    try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
        var resumed = false
        conn.stateUpdateHandler = { state in
            guard !resumed else { return }
            switch state {
            case .ready:
                resumed = true
                cont.resume()
            case .failed(let err):
                resumed = true
                cont.resume(throwing: TransferError.connectionFailed(err))
            case .cancelled:
                resumed = true
                cont.resume(throwing: CancellationError())
            default: break
            }
        }
        conn.start(queue: DispatchQueue.global(qos: .userInitiated))
    }
}

private func send(_ conn: NWConnection, data: Data) async throws {
    try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
        conn.send(content: data, completion: .contentProcessed { error in
            if let error = error {
                cont.resume(throwing: error)
            } else {
                cont.resume()
            }
        })
    }
}

/// Receive bytes (up to maxLength), returning whatever arrives.
private func receiveChunk(_ conn: NWConnection, maxLength: Int) async throws -> Data {
    try await withCheckedThrowingContinuation { cont in
        conn.receive(minimumIncompleteLength: 1, maximumLength: maxLength) { data, _, _, error in
            if let error = error {
                cont.resume(throwing: error)
            } else if let data = data, !data.isEmpty {
                cont.resume(returning: data)
            } else {
                cont.resume(throwing: ProtocolError.connectionClosed)
            }
        }
    }
}

/// Receive exactly `count` bytes, looping as needed.
private func receiveExact(_ conn: NWConnection, count: Int) async throws -> Data {
    var buffer = Data()
    while buffer.count < count {
        let chunk = try await receiveChunk(conn, maxLength: count - buffer.count)
        buffer.append(chunk)
    }
    return buffer
}

/// Read a 4-byte length-prefixed JSON message from the connection.
private func receiveMessage(_ conn: NWConnection) async throws -> TunnelMessage {
    let lengthBytes = try await receiveExact(conn, count: 4)
    let length = Int(UInt32(bigEndian: lengthBytes.withUnsafeBytes { $0.load(as: UInt32.self) }))
    guard length <= maxMessageSize else { throw ProtocolError.messageTooLarge(length) }
    let payload = try await receiveExact(conn, count: length)
    return try JSONDecoder().decode(TunnelMessage.self, from: payload)
}

// MARK: - Filename sanitization

private func sanitizeFilename(_ name: String) -> String {
    name.unicodeScalars
        .map { "/\\:*?\"<>|".unicodeScalars.contains($0) ? Character("_") : Character($0) }
        .map(String.init)
        .joined()
}

// MARK: - SHA256 hex helper

private extension SHA256Digest {
    var hexString: String {
        map { String(format: "%02x", $0) }.joined()
    }
}

