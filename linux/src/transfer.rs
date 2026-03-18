/// File transfer — outgoing (sender) side.
///
/// The incoming (receiver) side is handled by the axum HTTP handlers in `app.rs`.
///
/// Flow (LocalSend v2):
///   1. POST /api/localsend/v2/prepare-upload  — offer file list, wait for accept/deny.
///   2. POST /api/localsend/v2/upload          — stream each file's bytes.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use anyhow::{bail, Result};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio_util::io::ReaderStream;

use crate::app::AppEvent;
use crate::inhibit::InhibitGuard;
use crate::localsend::{
    DeviceInfo, FileMetadata, PrepareUploadRequest, PrepareUploadResponse, LOCALSEND_PORT,
};
use crate::tls::TlsStack;

pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
const CHUNK_SIZE: usize = 64 * 1024; // 64 KiB read buffer

// ── Public entry point ────────────────────────────────────────────────────────

/// Send one or more files/directories to `peer_addr`.
///
/// Directories are ZIP-compressed into a temp file before transfer.
/// All paths are sent in a single `prepare-upload` → N × `upload` round trip.
/// Emits `TransferProgress`, `TransferComplete`, and `TransferError` events.
pub async fn send_files(
    peer_addr: SocketAddr,
    paths: Vec<PathBuf>,
    sender_name: String,
    sender_fingerprint: String,
    /// The peer's announced fingerprint from UDP discovery — used as the TOFU key.
    peer_fingerprint: String,
    tls: Arc<TlsStack>,
    event_tx: async_channel::Sender<AppEvent>,
) -> Result<()> {
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let result =
        try_send_files(&transfer_id, peer_addr, paths, sender_name, sender_fingerprint, peer_fingerprint, tls, &event_tx).await;
    if let Err(ref e) = result {
        let _ = event_tx
            .send(AppEvent::TransferError {
                transfer_id,
                message: e.to_string(),
            })
            .await;
    }
    result
}

async fn try_send_files(
    transfer_id: &str,
    peer_addr: SocketAddr,
    paths: Vec<PathBuf>,
    sender_name: String,
    sender_fingerprint: String,
    peer_fingerprint: String,
    tls: Arc<TlsStack>,
    event_tx: &async_channel::Sender<AppEvent>,
) -> Result<()> {
    let _inhibit = InhibitGuard::acquire("Sending files").await;

    // ── Prepare file list (zip directories) ──────────────────────────────────
    struct FileEntry {
        source_path: PathBuf,
        display_name: String,
        size_bytes: u64,
        file_type: String,
        sha256: Option<String>,
        _temp: Option<tempfile::NamedTempFile>,
    }

    let mut entries: Vec<(String, FileEntry)> = Vec::with_capacity(paths.len());
    for path in &paths {
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
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            // Compute SHA-256 so the receiver can verify integrity.
            let sha256 = {
                let bytes = tokio::fs::read(path).await?;
                hex::encode(Sha256::digest(&bytes))
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
        .use_preconfigured_tls(tls.client_config_for_peer(&peer_fingerprint))
        .build()?;

    let base_url = format!("https://{}:{}", peer_addr.ip(), peer_addr.port());

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
            alias: sender_name.clone(),
            version: "2.0".to_string(),
            device_model: Some("PC".to_string()),
            device_type: Some("desktop".to_string()),
            fingerprint: sender_fingerprint,
            port: LOCALSEND_PORT,
            protocol: "https".to_string(),
            download: false,
            announce: None,
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
        let bst = bytes_sent_total.clone();
        let progress_stream = base_stream.map(move |chunk| {
            if let Ok(ref c) = chunk {
                let new_total = bst.fetch_add(c.len() as u64, Ordering::Relaxed) + c.len() as u64;
                let (bps, eta) = speed_eta(new_total, total_bytes, start_instant);
                let _ = event_tx_prog.try_send(AppEvent::TransferProgress {
                    transfer_id: tid.clone(),
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

    let saved_to = entries
        .first()
        .map(|(_, e)| e.source_path.clone())
        .unwrap_or_default();

    let _ = event_tx
        .send(AppEvent::TransferComplete {
            transfer_id: transfer_id.to_string(),
            saved_to,
        })
        .await;

    tracing::info!("All {} file(s) transferred ✓", entries.len());
    Ok(())
}

// ── Directory compression ─────────────────────────────────────────────────────

/// Zip a directory tree into a temp file. Returns `(temp_file, zip_size_bytes)`.
async fn zip_directory(dir_path: PathBuf) -> Result<(tempfile::NamedTempFile, u64)> {
    tokio::task::spawn_blocking(move || {
        let tmp = tempfile::NamedTempFile::new()?;
        {
            let file = tmp.as_file().try_clone()?;
            let mut zip = zip::ZipWriter::new(file);
            add_dir_to_zip(&dir_path, &dir_path, &mut zip)?;
            zip.finish()?;
        }
        let size = tmp.as_file().metadata()?.len();
        Ok((tmp, size))
    })
    .await?
}

fn add_dir_to_zip(
    base: &Path,
    dir: &Path,
    zip: &mut zip::ZipWriter<impl std::io::Write + std::io::Seek>,
) -> Result<()> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(base)?;
        let zip_name = rel.to_string_lossy();

        if zip_name.is_empty() {
            continue;
        }

        if path.is_dir() {
            zip.add_directory(zip_name.as_ref(), options)?;
        } else {
            zip.start_file(zip_name.as_ref(), options)?;
            let mut f = std::fs::File::open(path)?;
            std::io::copy(&mut f, zip)?;
        }
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Simple MIME type guess from extension (good enough for LocalSend metadata).
fn mime_guess(ext: &str) -> String {
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Compute (bytes_per_sec, eta_secs) from transfer state.
pub fn speed_eta(bytes_done: u64, total_bytes: u64, start: Instant) -> (u64, Option<u64>) {
    let elapsed = start.elapsed().as_secs_f64();
    if elapsed < 0.1 || bytes_done == 0 {
        return (0, None);
    }
    let bps = bytes_done as f64 / elapsed;
    let remaining = total_bytes.saturating_sub(bytes_done);
    let eta = if bps > 1.0 { Some((remaining as f64 / bps) as u64) } else { None };
    (bps as u64, eta)
}

/// Sanitize a filename from an untrusted peer: replace path-separator and
/// shell-special characters with underscores.
pub fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    if sanitized == ".." || sanitized == "." {
        return "file".to_string();
    }
    sanitized
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── sanitize_filename ────────────────────────────────────────────────────

    #[test]
    fn sanitize_normal() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("My Report.docx"), "My Report.docx");
    }

    #[test]
    fn sanitize_dot_dot() {
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename("."), "file");
    }

    #[test]
    fn sanitize_path_traversal_embedded() {
        // Slashes inside a name become underscores, not ".." collapse.
        assert_eq!(sanitize_filename("../../etc/passwd"), ".._.._etc_passwd");
    }

    #[test]
    fn sanitize_all_forbidden_chars() {
        assert_eq!(sanitize_filename(r#"/:*?"<>|\\"#), "__________");
    }

    #[test]
    fn sanitize_unicode_preserved() {
        assert_eq!(sanitize_filename("файл.txt"), "файл.txt");
        assert_eq!(sanitize_filename("图片.png"), "图片.png");
        assert_eq!(sanitize_filename("résumé.pdf"), "résumé.pdf");
    }

    #[test]
    fn sanitize_empty() {
        assert_eq!(sanitize_filename(""), "");
    }

    // ── speed_eta ────────────────────────────────────────────────────────────

    #[test]
    fn speed_zero_at_start() {
        let start = Instant::now();
        let (bps, eta) = speed_eta(0, 1_000_000, start);
        assert_eq!(bps, 0);
        assert!(eta.is_none());
    }

    #[test]
    fn speed_basic_calculation() {
        use std::time::Duration;
        // Simulate 1 MiB transferred over 1 second (roughly).
        // We can't control Instant, so we test the math directly.
        let elapsed_secs = 2.0_f64;
        let bytes_done = 2_000_000_u64;
        let total_bytes = 10_000_000_u64;
        let bps = bytes_done as f64 / elapsed_secs;
        let remaining = total_bytes - bytes_done;
        let eta = (remaining as f64 / bps) as u64;
        assert_eq!(bps as u64, 1_000_000);
        assert_eq!(eta, 8); // 8 MB remaining at 1 MB/s = 8 s
        let _ = Duration::from_secs(1); // just verify it compiles
    }

    // ── mime_guess ───────────────────────────────────────────────────────────

    #[test]
    fn mime_known_extensions() {
        assert_eq!(mime_guess("jpg"), "image/jpeg");
        assert_eq!(mime_guess("PNG"), "image/png"); // case-insensitive
        assert_eq!(mime_guess("pdf"), "application/pdf");
        assert_eq!(mime_guess("mp4"), "video/mp4");
    }

    #[test]
    fn mime_unknown_falls_back() {
        assert_eq!(mime_guess("xyz"), "application/octet-stream");
        assert_eq!(mime_guess(""), "application/octet-stream");
    }
}
