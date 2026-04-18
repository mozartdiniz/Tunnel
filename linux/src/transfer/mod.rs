/// File transfer — outgoing (sender) side.
///
/// The incoming (receiver) side is handled by the axum HTTP handlers in `app/handlers/`.
///
/// Flow (LocalSend v2):
///   1. POST /api/localsend/v2/prepare-upload  — offer file list, wait for accept/deny.
///   2. POST /api/localsend/v2/upload          — stream each file's bytes.
mod helpers;
mod zip;

pub use helpers::{sanitize_filename, sanitize_sync_path, speed_eta, MAX_FILE_SIZE};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use anyhow::{bail, Result};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

use crate::app::AppEvent;
use crate::inhibit::InhibitGuard;
use crate::localsend::{
    DeviceInfo, FileMetadata, PrepareUploadRequest, PrepareUploadResponse, LOCALSEND_PORT,
};
use crate::tls::TlsStack;

use helpers::{mime_guess, CHUNK_SIZE};
use zip::zip_directory;

/// Parameters for an outgoing file transfer.
pub struct SendRequest {
    pub peer_addr: SocketAddr,
    /// Peer's announced fingerprint from UDP discovery — used as TOFU key.
    pub peer_fingerprint: String,
    pub paths: Vec<PathBuf>,
    pub sender_name: String,
    pub sender_fingerprint: String,
    /// When set, this is a sync transfer: `file_name` in metadata carries the
    /// relative path from this root, and `DeviceInfo.sync` is set to `true`.
    pub sync_root: Option<PathBuf>,
}

/// Metadata and on-disk location for a single file to be sent.
struct FileEntry {
    source_path: PathBuf,
    display_name: String,
    size_bytes: u64,
    file_type: String,
    sha256: Option<String>,
    /// Kept alive to prevent the temp file from being deleted until we finish sending.
    _temp: Option<tempfile::NamedTempFile>,
}

/// Send one or more files/directories to `req.peer_addr`.
///
/// Directories are ZIP-compressed into a temp file before transfer.
/// All paths are sent in a single `prepare-upload` → N × `upload` round trip.
/// Emits `TransferProgress`, `TransferComplete`, and `TransferError` events.
pub async fn send_files(
    req: SendRequest,
    tls: Arc<TlsStack>,
    event_tx: async_channel::Sender<AppEvent>,
) -> Result<()> {
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let peer_fingerprint = req.peer_fingerprint.clone();
    let result = try_send_files(&transfer_id, req, tls, &event_tx).await;
    if let Err(ref e) = result {
        let _ = event_tx
            .send(AppEvent::TransferError {
                transfer_id,
                peer_fingerprint,
                message: e.to_string(),
            })
            .await;
    }
    result
}

async fn try_send_files(
    transfer_id: &str,
    req: SendRequest,
    tls: Arc<TlsStack>,
    event_tx: &async_channel::Sender<AppEvent>,
) -> Result<()> {
    let peer_fp = req.peer_fingerprint.clone();
    let _inhibit = InhibitGuard::acquire("Sending files").await;

    // ── Prepare file list (zip directories) ──────────────────────────────────
    let mut entries: Vec<(String, FileEntry)> = Vec::with_capacity(req.paths.len());
    for path in &req.paths {
        let file_id = uuid::Uuid::new_v4().to_string();
        let entry = if path.is_dir() {
            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("folder")
                .to_string();
            let zip_name = format!("{dir_name}.zip");
            let (tmp, size) = zip_directory(path.clone()).await?;
            FileEntry {
                source_path: tmp.path().to_path_buf(),
                display_name: zip_name,
                size_bytes: size,
                file_type: "application/zip".to_string(),
                sha256: None,
                _temp: Some(tmp),
            }
        } else {
            let size = tokio::fs::metadata(path).await?.len();
            if size > MAX_FILE_SIZE {
                bail!(
                    "File '{}' is too large ({size} bytes; limit is {MAX_FILE_SIZE})",
                    path.display()
                );
            }
            // For sync transfers, preserve the relative path from sync_root.
            let name = if let Some(ref root) = req.sync_root {
                path.strip_prefix(root)
                    .ok()
                    .and_then(|p| p.to_str())
                    .map(|s| s.replace(std::path::MAIN_SEPARATOR, "/"))
                    .unwrap_or_else(|| {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("file")
                            .to_string()
                    })
            } else {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string()
            };
            // Stream SHA-256 to avoid loading the entire file into memory.
            let sha256 = {
                let mut file = tokio::fs::File::open(path).await?;
                let mut hasher = Sha256::new();
                let mut buf = vec![0u8; CHUNK_SIZE];
                loop {
                    let n = file.read(&mut buf).await?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                hex::encode(hasher.finalize())
            };
            FileEntry {
                source_path: path.clone(),
                display_name: name,
                size_bytes: size,
                file_type: mime_guess(&path.extension().and_then(|e| e.to_str()).unwrap_or("")),
                sha256: Some(sha256),
                _temp: None,
            }
        };
        entries.push((file_id, entry));
    }

    let total_bytes: u64 = entries.iter().map(|(_, e)| e.size_bytes).sum();

    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls.client_config_for_peer(&req.peer_fingerprint))
        .build()?;

    let base_url = format!("https://{}:{}", req.peer_addr.ip(), req.peer_addr.port());

    // ── 1. prepare-upload ────────────────────────────────────────────────────
    let files_meta: HashMap<String, FileMetadata> = entries
        .iter()
        .map(|(id, e)| {
            (
                id.clone(),
                FileMetadata {
                    id: id.clone(),
                    file_name: e.display_name.clone(),
                    size: e.size_bytes,
                    file_type: e.file_type.clone(),
                    sha256: e.sha256.clone(),
                    preview: None,
                },
            )
        })
        .collect();

    let prepare_req = PrepareUploadRequest {
        info: DeviceInfo {
            alias: req.sender_name.clone(),
            version: "2.0".to_string(),
            device_model: Some("PC".to_string()),
            device_type: Some("desktop".to_string()),
            fingerprint: req.sender_fingerprint,
            port: LOCALSEND_PORT,
            protocol: "https".to_string(),
            download: false,
            announce: None,
            sync: req.sync_root.as_ref().map(|_| true),
        },
        files: files_meta,
    };

    tracing::info!("Sending prepare-upload ({} file(s)) to {base_url}", entries.len());

    let resp = client
        .post(format!("{base_url}/api/localsend/v2/prepare-upload"))
        .json(&prepare_req)
        .send()
        .await?;

    match resp.status().as_u16() {
        403 => bail!("Peer denied the transfer"),
        s if s != 200 => bail!("Unexpected response from peer: HTTP {s}"),
        _ => {}
    }

    let PrepareUploadResponse { session_id, files: tokens } = resp.json().await?;
    tracing::info!("Transfer accepted; uploading {total_bytes} bytes total");

    // ── 2. Upload each file ──────────────────────────────────────────────────
    let start_instant = Instant::now();
    let bytes_sent_total = Arc::new(AtomicU64::new(0));

    for (file_id, entry) in &entries {
        let token = tokens
            .get(file_id)
            .ok_or_else(|| {
                anyhow::anyhow!("Peer returned no token for file '{}'", entry.display_name)
            })?
            .clone();

        let file = tokio::fs::File::open(&entry.source_path).await?;
        let base_stream = ReaderStream::with_capacity(file, CHUNK_SIZE);

        // Wrap stream to emit TransferProgress per chunk with speed/ETA.
        let event_tx_prog = event_tx.clone();
        let tid = transfer_id.to_string();
        let pfp = peer_fp.clone();
        let bst = bytes_sent_total.clone();
        let progress_stream = base_stream.map(move |chunk| {
            if let Ok(ref c) = chunk {
                let new_total = bst.fetch_add(c.len() as u64, Ordering::Relaxed) + c.len() as u64;
                let (bps, eta) = speed_eta(new_total, total_bytes, start_instant);
                let _ = event_tx_prog.try_send(AppEvent::TransferProgress {
                    transfer_id: tid.clone(),
                    peer_fingerprint: pfp.clone(),
                    bytes_done: new_total,
                    total_bytes,
                    bytes_per_sec: bps,
                    eta_secs: eta,
                });
            }
            chunk
        });

        let file_size = entry.size_bytes;
        let upload_resp = client
            .post(format!("{base_url}/api/localsend/v2/upload"))
            .query(&[
                ("sessionId", session_id.as_str()),
                ("fileId", file_id.as_str()),
                ("token", token.as_str()),
            ])
            .header("Content-Type", "application/octet-stream")
            .header("Content-Length", file_size.to_string())
            .body(reqwest::Body::wrap_stream(progress_stream))
            .send()
            .await?;

        if !upload_resp.status().is_success() {
            bail!(
                "Upload failed for '{}': HTTP {}",
                entry.display_name,
                upload_resp.status()
            );
        }
        tracing::info!("Uploaded '{}' ✓", entry.display_name);
    }

    // Sender side: files were not saved locally, so saved_to is None.
    let _ = event_tx
        .send(AppEvent::TransferComplete {
            transfer_id: transfer_id.to_string(),
            peer_fingerprint: peer_fp,
            saved_to: None,
        })
        .await;

    tracing::info!("All {} file(s) transferred ✓", entries.len());
    Ok(())
}
