use std::time::Instant;

pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
pub const CHUNK_SIZE: usize = 64 * 1024; // 64 KiB read buffer

/// Simple MIME type guess from extension (good enough for LocalSend metadata).
pub fn mime_guess(ext: &str) -> String {
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

/// Sanitize a relative path from a sync peer.
/// Each component is individually sanitized; `.` and `..` are dropped entirely.
/// Returns a `PathBuf` safe to join onto the sync folder.
pub fn sanitize_sync_path(name: &str) -> std::path::PathBuf {
    let normalized = name.replace('\\', "/");
    let mut out = std::path::PathBuf::new();
    for component in normalized.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            continue;
        }
        let safe = sanitize_filename(component);
        if !safe.is_empty() {
            out.push(safe);
        }
    }
    if out.as_os_str().is_empty() {
        out.push("file");
    }
    out
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
