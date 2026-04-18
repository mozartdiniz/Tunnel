/// Shared types for the LocalSend open protocol v2.
///
/// Spec: https://github.com/localsend/protocol
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Fixed port used by all LocalSend-compatible apps.
pub const LOCALSEND_PORT: u16 = 53317;

/// IPv4 multicast group for peer discovery.
pub const MULTICAST_ADDR: &str = "224.0.0.167";

/// Device descriptor — used in UDP announcements and in the prepare-upload request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: String,
    pub download: bool,
    /// Present in UDP announcements only; absent in prepare-upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce: Option<bool>,
    /// Set to `true` to signal that this device has a sync folder configured.
    /// Other LocalSend clients that don't implement sync simply ignore this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<bool>,
}

/// Per-file metadata inside a prepare-upload request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub id: String,
    pub file_name: String,
    pub size: u64,
    pub file_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// POST /api/localsend/v2/prepare-upload  — sent by the file sender.
#[derive(Debug, Serialize, Deserialize)]
pub struct PrepareUploadRequest {
    pub info: DeviceInfo,
    /// fileId → FileMetadata
    pub files: HashMap<String, FileMetadata>,
}

/// 200 response to prepare-upload — sent by the receiver when accepting.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadResponse {
    pub session_id: String,
    /// fileId → upload token
    pub files: HashMap<String, String>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> DeviceInfo {
        DeviceInfo {
            alias: "Alice's Laptop".to_string(),
            version: "2.0".to_string(),
            device_model: Some("PC".to_string()),
            device_type: Some("desktop".to_string()),
            fingerprint: "abc123def456abc123def456abc123de".to_string(),
            port: 53317,
            protocol: "https".to_string(),
            download: false,
            announce: Some(true),
            sync: None,
        }
    }

    #[test]
    fn device_info_camelcase_keys() {
        let json = serde_json::to_string(&alice()).unwrap();
        assert!(json.contains("\"deviceModel\""), "must use camelCase: {json}");
        assert!(json.contains("\"deviceType\""), "must use camelCase: {json}");
        assert!(!json.contains("\"device_model\""), "snake_case leaked: {json}");
        assert!(!json.contains("\"device_type\""), "snake_case leaked: {json}");
    }

    #[test]
    fn device_info_roundtrip() {
        let info = alice();
        let json = serde_json::to_string(&info).unwrap();
        let back: DeviceInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.alias, info.alias);
        assert_eq!(back.fingerprint, info.fingerprint);
        assert_eq!(back.port, info.port);
        assert_eq!(back.announce, Some(true));
    }

    #[test]
    fn device_info_skip_none_fields() {
        let info = DeviceInfo {
            alias: "Bob".to_string(),
            version: "2.0".to_string(),
            device_model: None,
            device_type: None,
            fingerprint: "fp".to_string(),
            port: 53317,
            protocol: "https".to_string(),
            download: false,
            announce: None,
            sync: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("deviceModel"), "None fields must be absent: {json}");
        assert!(!json.contains("announce"), "None announce must be absent: {json}");
    }

    #[test]
    fn file_metadata_camelcase_and_roundtrip() {
        let meta = FileMetadata {
            id: "file-001".to_string(),
            file_name: "photo.jpg".to_string(),
            size: 1_048_576,
            file_type: "image/jpeg".to_string(),
            sha256: Some("deadbeef".to_string()),
            preview: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"fileName\""), "fileName must be camelCase: {json}");
        assert!(json.contains("\"fileType\""), "fileType must be camelCase: {json}");
        assert!(!json.contains("preview"), "None preview must be absent: {json}");

        let back: FileMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.file_name, "photo.jpg");
        assert_eq!(back.size, 1_048_576);
        assert_eq!(back.sha256.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn prepare_upload_roundtrip() {
        let mut files = HashMap::new();
        files.insert(
            "f1".to_string(),
            FileMetadata {
                id: "f1".to_string(),
                file_name: "doc.pdf".to_string(),
                size: 512,
                file_type: "application/pdf".to_string(),
                sha256: None,
                preview: None,
            },
        );
        let req = PrepareUploadRequest { info: alice(), files };
        let json = serde_json::to_string(&req).unwrap();
        let back: PrepareUploadRequest = serde_json::from_str(&json).unwrap();
        assert!(back.files.contains_key("f1"));
        assert_eq!(back.info.alias, "Alice's Laptop");
    }

    #[test]
    fn prepare_upload_response_camelcase() {
        let mut tokens = HashMap::new();
        tokens.insert("f1".to_string(), "tok-abc".to_string());
        let resp = PrepareUploadResponse { session_id: "sess-1".to_string(), files: tokens };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"sessionId\""), "sessionId must be camelCase: {json}");
        let back: PrepareUploadResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_id, "sess-1");
        assert_eq!(back.files.get("f1").map(String::as_str), Some("tok-abc"));
    }

    #[test]
    fn multicast_constants() {
        assert_eq!(LOCALSEND_PORT, 53317);
        // Verify the multicast address parses as a valid Ipv4Addr.
        let addr: std::net::Ipv4Addr = MULTICAST_ADDR.parse().unwrap();
        assert!(addr.is_multicast());
    }
}
