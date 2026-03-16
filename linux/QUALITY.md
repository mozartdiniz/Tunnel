# Code Quality Assessment — Tunnel (Linux)

**Overall grade: B+ / Solid foundation with notable gaps**
**Assessed by:** Senior Rust + GTK + GNOME engineer perspective
**Date:** 2026-03-16

---

## Architecture — Strong

The thread model is correct and idiomatic for this stack:
- Tokio runtime in a dedicated OS thread (`std::thread::spawn`)
- GTK owns the main thread
- `async-channel` as the bridge (right choice over `std::sync::mpsc`)
- `Rc<RefCell<>>` on the UI side (correct — GTK is single-threaded), `Arc<Mutex<>>` / `Arc<RwLock<>>` on the tokio side

Module boundaries are clean: `protocol`, `tls`, `transfer`, `discovery`, `app`, `ui` each have one job.

---

## Bugs

### BUG-1 — Sender name is always empty
**File:** `src/transfer.rs:61`
**Status:** fixed

```rust
sender_name: String::new(), // "filled in by receiver from mDNS / TLS identity"
```

The receiver never fills it in — it just uses what it received. The transfer request dialog shows a blank sender name. Fix: populate `sender_name` from `config.device_name` before sending the `Ask` message.

---

### BUG-2 — Icon search path is a hardcoded relative path
**File:** `src/ui/mod.rs:63`
**Status:** fixed (switched to built-in `system-search-symbolic`; custom SVG and path lookup removed)

The custom `search-spinner-symbolic.svg` was never found because `add_search_path` requires a proper `hicolor/scalable/<category>/` directory tree, not a flat directory. The Inkscape-generated SVG also used features (patterns, sodipodi namespace) that librsvg cannot render. Fix: use the standard `system-search-symbolic` icon which is always present in the GTK/GNOME icon theme.

---

### BUG-3 — File is read twice on send (double-read)
**File:** `src/transfer.rs:44-45`
**Status:** fixed

```rust
let checksum = checksum_file(&file_path).await?; // full read #1
// ... file is read again in the streaming loop below (full read #2)
```

The receiver computes checksum incrementally while receiving. The sender should do the same — compute on the fly during streaming, eliminating the first full read.

---

## Medium Issues

### MED-1 — `&PathBuf` should be `&Path`
**File:** `src/transfer.rs:283`
**Status:** open

```rust
async fn checksum_file(path: &PathBuf) // non-idiomatic
async fn checksum_file(path: &Path)    // correct
```

`&Path` is the borrowed form; accepting `&PathBuf` forces callers to have an owned `PathBuf`.

---

### MED-2 — Unchecked `as u32` cast
**File:** `src/protocol.rs:54`
**Status:** open

```rust
let len = json.len() as u32; // truncates silently if > 4 GB (impossible but unsafe)
```

Fix: `let len = u32::try_from(json.len())?;`

---

### MED-3 — Unbounded file size in receive_file
**File:** `src/transfer.rs:215`
**Status:** open

A malicious peer can claim `size_bytes = u64::MAX`. The receive loop runs until the connection drops, holding resources indefinitely. Add a maximum accepted file size constant and bail early if exceeded.

---

### MED-4 — mDNS hostname conflicts
**File:** `src/discovery.rs:29`
**Status:** open

```rust
let hostname = format!("{instance_name}.local.");
```

Using the user-defined display name as the mDNS hostname invites network-level collisions. The hostname should be machine-stable (system hostname + UUID suffix). The human-readable name already lives correctly in the `display_name` TXT property.

---

### MED-5 — TOFU keyed on IP address (fragile on DHCP)
**File:** `src/tls.rs:137`
**Status:** open

```rust
let key = server_name.to_str().to_string(); // e.g., "192.168.1.42"
```

If a peer's IP changes (DHCP renewal), the fingerprint lookup misses and the cert is blindly re-trusted as a new peer. The mDNS `fullname` (stable per instance name) would be a better key.

---

### MED-6 — `persist()` silently swallows write errors
**File:** `src/tls.rs:121`
**Status:** open

```rust
let _ = std::fs::write(&self.file, json); // error silently dropped
```

Fix: `if let Err(e) = std::fs::write(...) { tracing::warn!("Failed to persist TOFU peers: {e}"); }`

---

### MED-7 — `/etc/hostname` is non-portable
**File:** `src/config.rs:51`
**Status:** open

Reads `/etc/hostname` directly. Works on most Linux distros but is not the correct POSIX call. Use `gethostname()` from the `nix` crate or `hostname::get()` from the `hostname` crate.

---

## GNOME Integration Gaps

### GNOME-1 — Hardcoded Adwaita palette colors
**File:** `src/ui/mod.rs:157,274,284,291`
**Status:** open

```rust
status_dot.set_markup("<span color='#33d17a'>●</span>"); // hardcoded green
// also #3584e4 (accent blue), #e01b24 (error red)
```

Breaks high-contrast mode and future palette changes. Use the Adwaita named tokens: `@success_color`, `@accent_color`, `@error_color` via CSS, or replace the dot with `AdwStatusPage`-style indicators.

---

### GNOME-2 — Missing Flatpak/packaging assets
**Status:** open

No `.desktop` file, no AppStream `metainfo.xml`, no application icon in GResource format. Required for:
- App showing up in GNOME Shell app grid
- Listing in GNOME Software / Flathub
- Correct taskbar icon

App ID `dev.tunnel.Tunnel` is set correctly — assets just need to be wired to it.

---

### GNOME-3 — `tokio = full` feature flag
**File:** `Cargo.toml:13`
**Status:** open

```toml
tokio = { version = "1", features = ["full"] }
```

Compiles every tokio subsystem. Enumerate only what's needed:
`features = ["rt-multi-thread", "net", "fs", "io-util", "sync", "time", "macros"]`

---

### GNOME-4 — `Discovery` has no `Drop` impl
**File:** `src/discovery.rs`
**Status:** open

`ServiceDaemon` from `mdns-sd` spawns background threads. Without explicit `daemon.shutdown()`, those threads leak on process exit. Implement `Drop for Discovery` that calls `self.daemon.shutdown()`.

---

## Style / Refactor

### STYLE-1 — `build_main_window` is too large
**File:** `src/ui/mod.rs:25-226`
**Status:** open

~200 lines doing widget construction, signal wiring, and the event loop setup in one function. Extract helpers: `build_content_area`, `build_header_bar`, `setup_event_loop`.

---

### STYLE-2 — `run_network` command dispatch is too large
**File:** `src/app.rs:65-218`
**Status:** open

~150 lines. The `match cmd` block could be a separate `handle_command(cmd, ...)` function for clarity and testability.

---

### STYLE-3 — `sanitize_filename` is over-restrictive
**File:** `src/transfer.rs:297`
**Status:** open

Windows-forbidden chars (`?`, `*`, `<`, `>`, `"`) are valid on Linux. Only `/` and `\0` are truly invalid. Also doesn't guard against reserved names `.` and `..` (though `join` onto a flat sanitized name is safe here anyway).

---

## Summary Table

| Area | Grade | Notes |
|---|---|---|
| Architecture | A | Thread model and module separation are correct |
| Protocol | B+ | Good framing; missing file-size cap (MED-3) |
| TLS / Security | B | TOFU is sound; IP-keyed lookup fragile (MED-5) |
| Transfer logic | B | Double-read on send (BUG-3); incremental checksum on receive is correct |
| UI / GTK4 | B | Correct Adwaita usage; hardcoded colors (GNOME-1); icon path bug (BUG-2) |
| GNOME packaging | D | No `.desktop`, no AppStream, no GResource (GNOME-2) |
| Rust idioms | B+ | Minor `&Path` vs `&PathBuf` (MED-1); `as u32` cast (MED-2) |
| Error handling | B | `anyhow` used well; a few silent swallows (MED-6) |
