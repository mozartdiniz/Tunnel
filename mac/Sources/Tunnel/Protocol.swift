import Foundation

// MARK: - LocalSend v2 open protocol constants

let localsendPort: UInt16 = 53317
let multicastAddress = "224.0.0.167"

// MARK: - Device descriptor

/// Matches the LocalSend v2 wire format (camelCase JSON keys).
/// `protocol` is a Swift keyword, so we map it via CodingKeys.
/// Optional fields are omitted from JSON when nil (not sent as null).
struct DeviceInfo: Codable {
    var alias: String
    var version: String
    var deviceModel: String?
    var deviceType: String?
    var fingerprint: String
    var port: UInt16
    var protocolScheme: String
    var download: Bool
    var announce: Bool?

    enum CodingKeys: String, CodingKey {
        case alias, version, deviceModel, deviceType, fingerprint, port
        case protocolScheme = "protocol"
        case download, announce
    }

    init(alias: String, version: String, deviceModel: String? = nil, deviceType: String? = nil,
         fingerprint: String, port: UInt16, protocolScheme: String, download: Bool, announce: Bool? = nil) {
        self.alias = alias; self.version = version; self.deviceModel = deviceModel
        self.deviceType = deviceType; self.fingerprint = fingerprint; self.port = port
        self.protocolScheme = protocolScheme; self.download = download; self.announce = announce
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        alias        = try c.decode(String.self, forKey: .alias)
        version      = try c.decode(String.self, forKey: .version)
        deviceModel  = try c.decodeIfPresent(String.self, forKey: .deviceModel)
        deviceType   = try c.decodeIfPresent(String.self, forKey: .deviceType)
        fingerprint  = try c.decode(String.self, forKey: .fingerprint)
        port         = try c.decode(UInt16.self, forKey: .port)
        protocolScheme = try c.decode(String.self, forKey: .protocolScheme)
        download     = try c.decode(Bool.self, forKey: .download)
        announce     = try c.decodeIfPresent(Bool.self, forKey: .announce)
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(alias,          forKey: .alias)
        try c.encode(version,        forKey: .version)
        try c.encodeIfPresent(deviceModel, forKey: .deviceModel)
        try c.encodeIfPresent(deviceType,  forKey: .deviceType)
        try c.encode(fingerprint,    forKey: .fingerprint)
        try c.encode(port,           forKey: .port)
        try c.encode(protocolScheme, forKey: .protocolScheme)
        try c.encode(download,       forKey: .download)
        try c.encodeIfPresent(announce, forKey: .announce)
    }
}

// MARK: - File metadata

struct FileMetadata: Codable {
    var id: String
    var fileName: String
    var size: UInt64
    var fileType: String
    var sha256: String?
    var preview: String?

    enum CodingKeys: String, CodingKey {
        case id, fileName, size, fileType, sha256, preview
    }

    init(id: String, fileName: String, size: UInt64, fileType: String,
         sha256: String? = nil, preview: String? = nil) {
        self.id = id; self.fileName = fileName; self.size = size
        self.fileType = fileType; self.sha256 = sha256; self.preview = preview
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id       = try c.decode(String.self,  forKey: .id)
        fileName = try c.decode(String.self,  forKey: .fileName)
        size     = try c.decode(UInt64.self,  forKey: .size)
        fileType = try c.decode(String.self,  forKey: .fileType)
        sha256   = try c.decodeIfPresent(String.self, forKey: .sha256)
        preview  = try c.decodeIfPresent(String.self, forKey: .preview)
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id,       forKey: .id)
        try c.encode(fileName, forKey: .fileName)
        try c.encode(size,     forKey: .size)
        try c.encode(fileType, forKey: .fileType)
        try c.encodeIfPresent(sha256,   forKey: .sha256)
        try c.encodeIfPresent(preview,  forKey: .preview)
    }
}

// MARK: - Prepare-upload request / response

struct PrepareUploadRequest: Codable {
    var info: DeviceInfo
    var files: [String: FileMetadata]  // fileId → FileMetadata
}

struct PrepareUploadResponse: Codable {
    var sessionId: String
    var files: [String: String]        // fileId → upload token
}
