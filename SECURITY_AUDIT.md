# Security Audit Report: Tunnel App

## Status Summary (re-audited 2026-03-19)

The original audit was written against an older implementation. The current codebase (LocalSend v2 protocol, Rust/axum backend) has fixed three of the five original findings. One issue is a known protocol-level limitation with no viable fix at the application layer.

| # | Finding | Original Severity | Current Status |
|---|---------|------------------|----------------|
| 1 | Sender impersonation | Critical | ⚠️ KNOWN LIMITATION — protocol-level, cannot fix |
| 2 | No file size limit (DoS) | Medium/High | ✅ FIXED — 10 GiB hard limit in upload handler |
| 3 | Path traversal filenames | Medium | ✅ FIXED — `..` and `.` rejected in sanitizer |
| 4 | Checksum verified after write | Medium | ✅ FIXED — SHA-256 streamed, verified before atomic rename |
| 5 | Unauthenticated mDNS discovery | Low/Medium | ℹ️ UNCHANGED — inherent to mDNS, out of scope |

---

## Finding 1 — Sender Impersonation (Protocol Limitation)

**Severity:** Critical (unchanged)
**Status:** ⚠️ Known limitation of the LocalSend v2 protocol — no application-level fix is possible.

**Description:**
The LocalSend v2 protocol does not use mutual TLS. The receiver's TLS certificate is verified via TOFU (Trust On First Use), but the sender presents no certificate. The sender's alias and fingerprint are self-reported in the JSON body of the `/api/localsend/v2/prepare-upload` request. The server (`tls/stack.rs`) is correctly built with `.with_no_client_auth()` because mTLS is not part of the protocol specification.

**Confirmed by live test (2026-03-19):**
A test device sent a prepare-upload request with `"alias": "SECURITY_TEST_SPOOF"` and a fabricated fingerprint. The Tunnel UI displayed this spoofed alias in the confirmation dialog. The user is given the opportunity to deny the transfer, but cannot distinguish a real peer from an impersonator.

**Mitigation (user-facing):**
Users should only accept transfers from aliases they recognise. The alias shown in the dialog is what the sender claims — it is not cryptographically verified.

**Not fixable at application layer** without forking the protocol. A protocol-level fix would require mTLS with cross-device certificate exchange during pairing — this is outside the scope of LocalSend v2 compatibility.

---

## Finding 2 — File Size Limit (DoS Protection)

**Status:** ✅ FIXED

**Fix location:** `linux/src/app/handlers/upload.rs`

```rust
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB

if file_meta.size > MAX_FILE_SIZE {
    return Err(StatusCode::PAYLOAD_TOO_LARGE);
}
```

The upload handler also checks available disk space before writing. Requests declaring a file larger than 10 GiB are rejected with HTTP 413 before any data is written to disk.

**Confirmed by live test (2026-03-19):** A prepare-upload declaring 10 GiB + 1 byte was accepted by the UI (user must accept), then the subsequent upload request was rejected with HTTP 413. ✅ PASS

---

## Finding 3 — Path Traversal Filenames

**Status:** ✅ FIXED

**Fix location:** `linux/src/transfer/helpers.rs` — `sanitize_filename()`

The sanitizer now handles the two reserved names that the original audit found were passed through unchanged:

- `".."` → `"file"`
- `"."` → `"file"`
- Path separators (`/`, `\`) → `"_"`
- Combined traversal like `"../../etc/passwd"` → `".._.._etc_passwd"`

**Confirmed by live test (2026-03-19):** All four dangerous filenames (`..`, `.`, `../../etc/passwd`, `../secret.txt`) were either sanitized to safe names or written as `file`. No writes outside the downloads folder. ✅ PASS

---

## Finding 4 — Checksum Verification (Write-then-Verify Flaw)

**Status:** ✅ FIXED

**Fix location:** `linux/src/app/handlers/upload.rs`

The upload handler now:
1. Streams the uploaded bytes into a **temporary file** while computing SHA-256 incrementally.
2. After the stream ends, compares the computed hash against the declared `sha256` in the file metadata.
3. If they match: atomically renames the temp file to its final destination.
4. If they mismatch: deletes the temp file — **nothing is written to the downloads folder**.

```rust
// (lines ~132-145 in upload.rs)
let computed = hasher.finalize();
if computed.as_slice() != expected_hash {
    fs::remove_file(&tmp_path).ok();
    return Err(StatusCode::INTERNAL_SERVER_ERROR);
}
fs::rename(&tmp_path, &final_path)?;
```

**Confirmed by live test (2026-03-19):** A file declared with SHA-256 of `"hello"` but uploaded with bytes `"WRONG"` was rejected with HTTP 500. The temp file was cleaned up; nothing appeared in the downloads folder. ✅ PASS

---

## Finding 5 — Unauthenticated mDNS Discovery

**Status:** ℹ️ Unchanged — inherent to mDNS, not a fix target.

Peer discovery uses UDP multicast on `224.0.0.167:53317`. Any device on the local network can advertise itself. This is by design in LocalSend v2 and is common to all mDNS-based discovery systems. The security boundary is the transfer confirmation dialog, not the discovery layer.

---

## Recommendations (remaining)

1. **User education:** Surface a note in the UI that the sender alias is self-reported and unverified. This is the only mitigation available within LocalSend v2 protocol constraints.

2. **Known-device list (future):** Implement optional device pairing / allowlist so recurring trusted peers are distinguished visually from unknown senders. This would not prevent spoofing but would make it more visible.

3. **mDNS (no action required):** Unauthenticated discovery is inherent to the protocol. The confirmation dialog is the correct security gate.
