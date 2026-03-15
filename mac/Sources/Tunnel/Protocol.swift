import Foundation

// Wire protocol matching the Linux Rust implementation exactly.
// Messages are length-prefixed JSON: 4-byte big-endian length + JSON payload.
// Enum tag field is "type" with SCREAMING_SNAKE_CASE values.

let protocolVersion: UInt8 = 1
let maxMessageSize = 1024 * 1024  // 1 MB

enum TunnelMessage: Codable {
    case ask(version: UInt8, transferId: String, senderName: String, fileName: String, sizeBytes: UInt64)
    case response(version: UInt8, status: ResponseStatus)
    case done(checksumSha256: String)

    enum MessageType: String, Codable {
        case ask     = "ASK"
        case response = "RESPONSE"
        case done    = "DONE"
    }

    enum CodingKeys: String, CodingKey {
        case type
        case version
        case transferId  = "transfer_id"
        case senderName  = "sender_name"
        case fileName    = "file_name"
        case sizeBytes   = "size_bytes"
        case status
        case checksumSha256 = "checksum_sha256"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type_ = try c.decode(MessageType.self, forKey: .type)
        switch type_ {
        case .ask:
            self = .ask(
                version: try c.decode(UInt8.self, forKey: .version),
                transferId: try c.decode(String.self, forKey: .transferId),
                senderName: try c.decode(String.self, forKey: .senderName),
                fileName: try c.decode(String.self, forKey: .fileName),
                sizeBytes: try c.decode(UInt64.self, forKey: .sizeBytes)
            )
        case .response:
            self = .response(
                version: try c.decode(UInt8.self, forKey: .version),
                status: try c.decode(ResponseStatus.self, forKey: .status)
            )
        case .done:
            self = .done(checksumSha256: try c.decode(String.self, forKey: .checksumSha256))
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .ask(let version, let transferId, let senderName, let fileName, let sizeBytes):
            try c.encode(MessageType.ask, forKey: .type)
            try c.encode(version, forKey: .version)
            try c.encode(transferId, forKey: .transferId)
            try c.encode(senderName, forKey: .senderName)
            try c.encode(fileName, forKey: .fileName)
            try c.encode(sizeBytes, forKey: .sizeBytes)
        case .response(let version, let status):
            try c.encode(MessageType.response, forKey: .type)
            try c.encode(version, forKey: .version)
            try c.encode(status, forKey: .status)
        case .done(let checksum):
            try c.encode(MessageType.done, forKey: .type)
            try c.encode(checksum, forKey: .checksumSha256)
        }
    }
}

enum ResponseStatus: String, Codable {
    case accepted    = "ACCEPTED"
    case denied      = "DENIED"
    case checksumOk  = "CHECKSUM_OK"
    case checksumFail = "CHECKSUM_FAIL"
}

// MARK: - Frame read/write helpers

enum ProtocolError: Error {
    case messageTooLarge(Int)
    case connectionClosed
    case unexpectedMessage(String)
}

/// Encode a message to length-prefixed JSON bytes.
func encodeMessage(_ msg: TunnelMessage) throws -> Data {
    let json = try JSONEncoder().encode(msg)
    var length = UInt32(json.count).bigEndian
    var frame = Data(bytes: &length, count: 4)
    frame.append(json)
    return frame
}

/// Decode a message from a 4-byte length prefix + JSON payload.
func decodeMessage(from data: Data) throws -> TunnelMessage {
    guard data.count >= 4 else { throw ProtocolError.connectionClosed }
    let length = Int(UInt32(bigEndian: data.subdata(in: 0..<4).withUnsafeBytes { $0.load(as: UInt32.self) }))
    guard length <= maxMessageSize else { throw ProtocolError.messageTooLarge(length) }
    guard data.count >= 4 + length else { throw ProtocolError.connectionClosed }
    return try JSONDecoder().decode(TunnelMessage.self, from: data.subdata(in: 4..<(4 + length)))
}
