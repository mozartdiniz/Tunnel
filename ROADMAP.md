# Tunnel — Roadmap to Production Quality

Comparison baseline: **Warp** (GNOME Circle, production).
Goal: match Warp's integration depth while keeping Tunnel's AirDrop-style UX advantage.
Strategic positioning: **the GNOME-native client for the LocalSend protocol**.

Items are grouped by area and ordered roughly by dependency (things earlier in each section
unblock things later). Each item notes whether it is **required** for a proper GNOME app,
**nice to have**, or a **feature gap** relative to Warp.

---

## 0. Protocol — adopt LocalSend  `[foundational]`

This is the most impactful single decision in the roadmap. Everything else builds on top of it.

### 0.1 — Why LocalSend instead of the current custom protocol

Tunnel's current protocol (`protocol.rs` + `tls.rs` + `discovery.rs`) is sound but has two
structural problems: it is undocumented (a black box to anyone who hasn't read the Rust
source), and it only works with other Tunnel clients — no mobile reach, no interoperability.

**LocalSend** is an open, published protocol (github.com/localsend/protocol) designed for
exactly the same use case: zero-config LAN file transfer with auto-discovery. Adopting it:

- Gives Tunnel immediate interoperability with ~50M existing LocalSend users on Android,
  iOS, Windows, and macOS — without them installing anything new
- Eliminates NIH criticism before it arises
- Transfers protocol maintenance to the LocalSend team
- Delivers multi-file and cancellation support for free (both are native in the spec)

The LocalSend protocol cannot be replaced by Magic Wormhole for this use case — Wormhole
requires a rendezvous server and manual code exchange, which is fundamentally incompatible
with zero-config auto-discovery. LocalSend shares Tunnel's architecture.

The niche this creates: LocalSend's Linux client is Flutter with GTK3 hacks — not a native
GNOME app. Tunnel becomes the GTK4/libadwaita member of the LocalSend ecosystem, the same
way Fractal is the GNOME-native Matrix client.

---

### 0.2 — What changes in the codebase

**Replaced entirely:**
- `src/protocol.rs` — custom binary wire format → LocalSend REST API over HTTPS
- `src/discovery.rs` — mDNS `_tunnel-p2p._tcp.local.` → UDP multicast 224.0.0.167:53317

**Adapted:**
- `src/tls.rs` — LocalSend uses one-way HTTPS (server cert only), not mutual TLS.
  The self-signed cert generation stays; the `TofuClientVerifier` (client cert requirement)
  is removed. TOFU fingerprint persistence is kept as a Tunnel enhancement (see 0.3).
- `src/transfer.rs` — file streaming logic stays; the framing changes from raw bytes to
  HTTP multipart upload/download.
- `src/app.rs` — the TCP listener is replaced by an HTTP server (see below).

**New dependencies:**
```toml
axum = { version = "0.8", features = ["multipart"] }  # HTTP server (receive side)
reqwest = { version = "0.12", features = ["multipart", "rustls-tls"], default-features = false }  # HTTP client (send side)
tokio-util = { version = "0.7", features = ["io"] }   # streaming body from file
```

**LocalSend endpoints to implement (receiver side, hosted by Tunnel):**

| Endpoint | Method | Purpose |
|---|---|---|
| `/api/localsend/v2/prepare-upload` | POST | Receive file metadata, return session + tokens |
| `/api/localsend/v2/upload` | POST | Receive file bytes (one request per file) |
| `/api/localsend/v2/cancel` | POST | Sender cancels mid-transfer |
| `/api/localsend/v2/info` | GET | Return device info (alias, fingerprint, version) |

**LocalSend flow (sender side, Tunnel calls out):**
1. Send POST to `/api/localsend/v2/prepare-upload` with file list metadata
2. Receiver shows accept/deny dialog; returns session ID + per-file tokens if accepted
3. For each file: POST to `/api/localsend/v2/upload?sessionId=…&fileId=…&token=…` with binary body
4. On cancel: POST to `/api/localsend/v2/cancel?sessionId=…`

**Discovery change:**
LocalSend devices announce themselves by sending a UDP multicast packet to 224.0.0.167:53317
containing a JSON payload with alias, device type, fingerprint, port, and protocol version.
Peers respond via TCP. Replace the `mdns-sd` crate with a simple UDP socket (tokio's
`UdpSocket` with `join_multicast_v4`); no external crate needed.

---

### 0.3 — Keep TOFU fingerprint persistence as a Tunnel enhancement

LocalSend currently has no TOFU implementation (open issue #2430 in their repo). Tunnel
can lead here: after the TLS handshake, store the peer's cert fingerprint on first contact
and warn the user if it changes on subsequent connections.

This is a pure addition on top of the LocalSend protocol — no protocol changes needed, no
incompatibility with other LocalSend clients. Other LocalSend clients simply won't do the
check; Tunnel will.

The existing `TofuStore` in `tls.rs` can be reused with minimal changes.

---

### 0.4 — LocalSend URI scheme  `[nice to have, replaces item 2.5]`

LocalSend defines a URI format for sharing transfer details out-of-band. Register
`x-scheme-handler/localsend` in the `.desktop` file. This replaces the previously
proposed custom `tunnel-transfer://` URI scheme and achieves interoperability with the
broader ecosystem.

---

## 1. Build system

### 1.1 — Replace `install.sh` with Meson  `[required]`

`install.sh` is a hand-rolled shell script. It works for a developer install but is
incompatible with Flatpak, distribution packages, and all standard GNOME tooling.

Warp uses Meson as its build system. Meson:
- drives `cargo build` as a sub-command via a cargo wrapper
- installs the binary, icons, `.desktop`, metainfo, GResource, and translations in one step
- is the expected entry point for Flatpak builders, CI, and distro packagers

**What to create:**
- `meson.build` at repo root — declares project, pulls in `i18n`, `gnome` modules,
  calls `subdir()` for `data/`, `po/`, `src/`
- `meson_options.txt` — at minimum a `profile` option (`development` / `release`)
  that controls the application ID suffix (`.Devel` in dev builds)
- `src/meson.build` — invokes `cargo` and installs the binary
- `data/meson.build` — installs desktop file, metainfo, icons, GResource, D-Bus service
- `po/meson.build` — drives `msgfmt` for each locale
- `build-aux/` — cargo wrapper script, Flatpak manifest, `dist-vendor.sh`

Until Meson is in place, items 1.2–1.5 below cannot be wired up properly.

---

### 1.2 — Development profile / application ID suffix  `[required]`

Warp builds under `app.drey.Warp.Devel` when `profile=development`. This allows
running a dev build alongside the installed release without them conflicting (separate
data dirs, separate D-Bus names, devel-stripe window decoration).

Tunnel's current app ID is `dev.tunnel.Tunnel`. The Meson build should append `.Devel`
for development builds, and the Rust code should read the app ID from a compile-time
environment variable set by Meson.

---

## 2. GNOME ecosystem assets

### 2.1 — AppStream metainfo  `[required]`

**File to create:** `data/dev.tunnel.Tunnel.metainfo.xml.in`

Required for:
- listing in GNOME Software and Flathub
- `appstreamcli validate` in CI
- `gnome-software` showing screenshots, description, and changelog

Must include:
- `<id>dev.tunnel.Tunnel</id>`
- `<name>`, `<summary>`, `<description>` (with feature bullet list)
- `<launchable type="desktop-id">dev.tunnel.Tunnel.desktop</launchable>`
- `<url>` entries: homepage, bugtracker, vcs-browser
- `<branding>` primary colors (light/dark) — used by GNOME Software for the app tile
- `<content_rating type="oars-1.1" />` — required by Flathub
- `<releases>` — at minimum one `<release>` entry per published version
- `<screenshots>` — at least one screenshot (Flathub requirement)
- `<requires>` / `<recommends>` — minimum display size, input controls

Warp's metainfo (`data/app.drey.Warp.metainfo.xml.in.in`) is the reference. Note the
double `.in.in` — the outer pass substitutes build variables (app ID, URLs), the inner
pass substitutes i18n strings.

---

### 2.2 — `.desktop` file  `[required]`

**File to create:** `data/dev.tunnel.Tunnel.desktop.in`

The current `install.sh` generates a minimal desktop entry by hand. A proper `.desktop`
file should be a source file validated by `desktop-file-validate` during the build.

Must include:
- `Exec=tunnel %u` — the `%u` enables URI handling (see item 2.5)
- `Icon=dev.tunnel.Tunnel`
- `Categories=Network;FileTransfer;`
- `DBusActivatable=true` — required for D-Bus activation (item 2.4)
- `StartupNotify=true`
- `StartupWMClass=tunnel` — must match the GTK application class
- `MimeType=` — list any URI schemes Tunnel handles (see item 2.5)
- `X-SingleMainWindow=true` — GNOME Shell only opens one window per click
- Translatable `Name=` and `Comment=` fields

---

### 2.3 — Application icon set  `[required]`

Tunnel currently ships one `app_icon.png` (512 × 512).

A complete icon set for a GNOME application requires:
- `dev.tunnel.Tunnel.svg` — full-colour vector source in `hicolor/scalable/apps/`
- `dev.tunnel.Tunnel-symbolic.svg` — monochrome symbolic variant in
  `hicolor/symbolic/apps/` — used by GNOME Shell search, task switcher, and
  notification badges
- The SVG files must be compatible with librsvg: no Inkscape-private namespaces
  (`sodipodi:`, `inkscape:`), no `<filter>` elements, no CSS `var()`, no patterns.
  Use plain `fill` attributes.

Warp ships both variants (`app.drey.Warp.svg` and `app.drey.Warp-symbolic.svg`) and
bundles them via GResource (see 2.6) as well as installing them into `hicolor`.

---

### 2.4 — D-Bus activation service file  `[required]`

**File to create:** `data/dev.tunnel.Tunnel.service.in`

```ini
[D-BUS Service]
Name=dev.tunnel.Tunnel
Exec=@bindir@/tunnel --gapplication-service
```

Without this file, clicking the app icon in GNOME Shell while it is already running will
open a second instance. With D-Bus activation, the shell sends the activate signal to the
existing instance instead. Required for `DBusActivatable=true` in the desktop file to work.

The binary must handle `--gapplication-service` by calling
`gio::Application::run_with_args()` correctly — GTK4 + libadwaita apps do this
automatically when using `adw::Application`.

---

### 2.5 — LocalSend URI scheme handler  `[nice to have]`

See item 0.4. Register `x-scheme-handler/localsend` in the `.desktop` file so that
clicking a LocalSend URI opens Tunnel directly. This also makes Tunnel compatible with
any future LocalSend ecosystem tooling that generates transfer URIs.

---

### 2.6 — GResource bundle  `[required]`

Warp bundles its CSS, UI files, and scalable icons into a GResource binary embedded in
the executable. Tunnel currently reads assets from the filesystem at runtime.

**File to create:** `data/resources/dev.tunnel.Tunnel.gresource.xml`

Should include:
- `style.css` — application CSS (see item 3.2 on removing hardcoded colors)
- Any symbolic SVG icons that the app uses internally (e.g. a Tunnel-specific symbolic
  icon for the peer list, status indicators)
- Any `.ui` files if Tunnel migrates to Blueprint/XML UI definitions in the future

Meson's `gnome.compile_resources()` compiles the XML into a `.gresource` file that
is linked into the binary. The Rust code then calls `gio::resources_register_include!()`
in `main.rs`.

Benefits: the installed binary is self-contained; no runtime path lookup required;
works identically inside and outside Flatpak.

---

## 3. UI polish

### 3.1 — Remove hardcoded Adwaita palette colors  `[required]`

**File:** `src/ui/mod.rs` — lines with `#33d17a`, `#3584e4`, `#e01b24`

Hardcoded hex colors break:
- high-contrast accessibility mode
- dark/light theme switching (Adwaita's named colors adjust automatically)
- future palette changes in libadwaita

**Fix:** Load `style.css` from GResource and use Adwaita's named color tokens:
- `@success_color` instead of `#33d17a`
- `@accent_color` instead of `#3584e4`
- `@error_color` instead of `#e01b24`

Alternatively, replace the status dot `●` with `adw::StatusPage` indicators or
`gtk::Image` with the appropriate symbolic icon — no CSS required.

---

### 3.2 — Transfer cancellation  `[required]`

Warp supports clean mid-transfer cancellation. Tunnel has no cancellation path once a
transfer starts.

With the LocalSend protocol adoption (item 0.2), cancellation is built into the spec:
the sender calls `POST /api/localsend/v2/cancel?sessionId=…` and the receiver deletes
the in-progress `.tmp` file. The Cancel button in the UI just needs to trigger that call.
No custom cancellation mechanism needs to be designed.

---

### 3.3 — Transfer speed and ETA display  `[nice to have]`

Warp shows bytes/sec and estimated time remaining. Tunnel's progress view only shows
bytes transferred / total. A rolling average over the last 2–3 seconds of chunks gives
a stable speed figure without much code.

---

### 3.4 — Sleep inhibition during transfer  `[nice to have]`

Warp calls the `org.gnome.SessionManager.Inhibit` D-Bus method (via `zbus`) while a
transfer is running to prevent the system from sleeping mid-transfer. This is a GNOME
best practice for any app that performs background I/O.

In Rust: use the `zbus` crate (already a transitive dependency via GTK/GIO) to call
`org.freedesktop.portal.Inhibit` (the portal-safe version that works in Flatpak).

---

### 3.5 — "Open containing folder" after transfer  `[nice to have]`

Warp opens the download folder in Nautilus when the user clicks the completed transfer
notification. Implement via `gio::AppInfo::launch_default_for_uri()` with a `file://`
URI pointing to the download directory.

---

### 3.6 — Keyboard shortcuts dialog  `[nice to have]`

Warp ships `data/resources/shortcuts-dialog.ui` and wires it to `<Control>question`.
GNOME HIG expects this in any app with non-trivial keyboard interactions. Low effort:
one `.ui` file + one signal handler.

---

### 3.7 — Multi-file and folder transfer  `[free with LocalSend adoption]`

Tunnel is currently single-file only. With LocalSend protocol adoption (item 0.2) this
becomes a UI task, not a protocol task — the spec natively supports sending a list of
files in a single `prepare-upload` request, with one `upload` call per file.

For folders: ZIP the directory into a temp archive before sending (same approach as
Warp), then send the archive as a single file. The receiver does not need to know it
is a ZIP; the user sees the archive in their Downloads folder and can extract it.

No protocol changes. No `is_archive` flag. Just UI work to accept multiple files or a
folder from the file chooser and iterate through the upload calls.

---

## 4. System notifications

### 4.1 — GNotification on transfer complete / incoming request  `[required]`

Tunnel does not send any desktop notifications. Warp notifies the user when:
- a transfer is received (with an action button to open the folder)
- a transfer completes on the sender side

Use `gio::Application::send_notification()` — it works inside Flatpak (routed through
the notification portal) and supports action buttons natively.

For incoming transfer requests specifically, the notification must be **actionable**:
- "Accept" button — maps to a GAction that resolves the pending decision channel
- "Decline" button — maps to a GAction that rejects

This is especially important when the main window is not in focus.

---

## 5. Internationalisation

### 5.1 — gettext integration  `[required for Flathub / GNOME Circle]`

Tunnel has no i18n. All UI strings are hardcoded in Rust.

Warp ships 46 `.po` files and a full gettext pipeline driven by Meson.

Steps:
1. Wrap all user-visible strings in `gettext("…")` / `ngettext()` calls in Rust using
   the `gettextrs` crate (the Rust binding for libintl, used by Warp)
2. Create `po/POTFILES.in` listing all `.rs` source files
3. Create `po/LINGUAS` listing supported locales
4. Run `xgettext` (or `build-aux/generate-potfile.sh`) to extract a `.pot` template
5. Submit the `.pot` to GNOME's Damned Lies translation platform, or manage `.po` files
   manually

At minimum, English-only is fine to start — the infrastructure just needs to be in place
so translators can contribute without code changes.

---

## 6. Flatpak

### 6.1 — Flatpak manifest  `[required for Flathub]`

**File to create:** `build-aux/dev.tunnel.Tunnel.yaml` (or `.json`)

The manifest declares:
- runtime: `org.gnome.Platform` (currently version 48)
- SDK: `org.gnome.Sdk`
- `finish-args`: the portal permissions Tunnel needs:
  - `--share=network` — TCP connections
  - `--filesystem=xdg-download` — write to Downloads folder
  - `--talk-name=org.freedesktop.portal.FileChooser` — file open dialog (portal)
  - `--talk-name=org.freedesktop.portal.Inhibit` — sleep inhibition
  - `--talk-name=org.freedesktop.Notifications` — desktop notifications
- `modules`: the Tunnel source module, built with Meson

Note: inside Flatpak, direct filesystem reads outside the sandbox are blocked. Tunnel
must use the `ashpd` or `zbus` crates (or GIO portals directly) for:
- file open/save dialogs (FileChooser portal)
- sleep inhibit (Inhibit portal)

Warp's `build-aux/app.drey.Warp.yaml` and `build-aux/app.drey.Warp.Devel.json` are the
direct reference.

---

### 6.2 — Vendor dependencies for offline Flatpak build  `[required for Flatpak CI]`

Flatpak builds have no network access. Cargo dependencies must be vendored.

Warp uses `build-aux/dist-vendor.sh` and `build-aux/flatpak-cargo-generator.py` to
produce a `cargo-sources.json` that the Flatpak manifest can consume.

The same scripts can be copied verbatim; they are generic.

---

## 7. Code quality (remaining open items from QUALITY.md)

These are not GNOME-integration issues but should be resolved before a 1.0 release.

| ID | File | Issue |
|---|---|---|
| MED-1 | `transfer.rs` | `&PathBuf` parameter should be `&Path` |
| ~~MED-2~~ | ~~`protocol.rs:54`~~ | ~~`json.len() as u32` unchecked cast~~ — **obsolete**: `protocol.rs` is replaced by LocalSend HTTP API (item 0.2) |
| MED-4 | `discovery.rs` | Display name used as network identifier — superseded by LocalSend's `alias` field, but still apply the fix to the UDP announcement payload |
| MED-6 | `tls.rs` | `persist()` silently drops write errors; log a warning instead |
| MED-7 | `config.rs:51` | `/etc/hostname` is non-portable; use the `gethostname` crate (also required by Windows port, item 9.1) |
| STYLE-1 | `ui/mod.rs` | `build_main_window` (~200 lines); extract `build_content_area`, `build_header_bar`, `setup_event_loop` |
| STYLE-2 | `app.rs` | `run_network` command dispatch (~150 lines); extract `handle_command()` |
| GNOME-3 | `Cargo.toml` | `tokio = { features = ["full"] }` — enumerate only needed features |
| GNOME-4 | `discovery.rs` | **Obsolete after 0.2**: `mdns-sd` crate removed; new UDP multicast socket does not leak threads |

---

## 8. Help documentation

### 8.1 — GNOME Help pages  `[nice to have]`

Warp ships HTML/Mallard help pages in `help/C/` (and translations under `help/<locale>/`),
wired to `<F1>` via `gtk::UriLauncher` or `gio::AppInfo`. GNOME Software surfaces these
in the app's "Help" button.

Not required for Flathub, but expected in GNOME Circle applications.

---

## 9. Windows port

The macOS version of Tunnel remains a native Swift app. This section covers Windows only.

Warp's Windows strategy is directly applicable to Tunnel. The approach is cross-compilation
from Linux using MinGW — no Windows development machine required.

### 9.1 — Replace Linux-only dependencies  `[required]`

Tunnel has exactly two Linux-specific blockers:

**`nix::statvfs` in `transfer.rs:229`**

`nix` is Unix-only. Replace with the `fs2` crate, which wraps `GetDiskFreeSpaceEx` on
Windows and `statvfs` on Unix behind a single API:

```toml
# Cargo.toml — replace:
nix = { version = "0.29", features = ["fs"] }
# with:
fs2 = "0.4"
```

```rust
// transfer.rs
use fs2::available_space;
let available = available_space(&config.download_dir)?;
```

**`/etc/hostname` in `config.rs:51`**

Already flagged as MED-7. Replace with the `gethostname` crate:

```toml
gethostname = "0.4"
```

```rust
// config.rs
let hostname = gethostname::gethostname().to_string_lossy().to_string();
```

Both changes also improve the Linux build (the `nix` dependency shrinks; the hostname
read becomes portable). They should be done regardless of the Windows port.

---

### 9.2 — Windows entry point and console handling  `[required]`

Directly copied from Warp:

```rust
// main.rs — top of file
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
```

```toml
# Cargo.toml
[target.'cfg(windows)'.dependencies]
win32console = "0.1"
```

```rust
// main.rs — call before gtk init
#[cfg(target_os = "windows")]
fn windows_hacks() {
    // Free the console opened by the OS, then re-attach to the parent terminal
    // so that stdout/stderr work when launched from a shell.
    let _ = win32console::console::WinConsole::free_console();
    let _ = win32console::console::WinConsole::attach_console(0xFFFFFFFF);
}
```

---

### 9.3 — Platform-conditional "open folder" after transfer  `[required]`

Linux uses `gio::AppInfo::launch_default_for_uri()` (item 3.5 in this roadmap).
Windows does not have GIO file launchers; use `explorer.exe /select,<path>` instead:

```rust
#[cfg(target_os = "windows")]
fn reveal_file(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

#[cfg(not(target_os = "windows"))]
fn reveal_file(path: &std::path::Path) {
    // gio::AppInfo::launch_default_for_uri (item 3.5)
}
```

---

### 9.4 — Linux-only dependencies gated behind `cfg(unix)`  `[required]`

Once items 9.1–9.3 are done, any remaining Unix-only crates must be feature-gated.
The only one at that point is `ashpd` (portal support, added in item 6.1 of this
roadmap). Following Warp's pattern:

```toml
[target.'cfg(unix)'.dependencies]
ashpd = { version = "0.12", features = ["gtk4", "async-std"], default-features = false }
```

---

### 9.5 — Meson build target for Windows  `[required]`

The Meson build (item 1.1) needs a `target` option so the CI can cross-compile:

```meson
# meson_options.txt
option('target', type: 'string', value: '',
       description: 'Cross-compilation target. Leave empty for native.')
```

```meson
# src/meson.build
if get_option('target') != ''
  cargo_options += ['--target', get_option('target')]
  if get_option('target').contains('windows')
    file_extension = '.exe'
  endif
endif
```

Build invocation (from CI or locally with Docker):
```bash
meson setup build -Dprefix="$PWD/package" -Dtarget="x86_64-pc-windows-gnu"
ninja -C build
ninja -C build install
```

---

### 9.6 — Application icon as `.ico`  `[required]`

Windows requires an `.ico` file embedded in the `.exe`. Convert the existing SVG at
build time (same pipeline as Warp):

```bash
rsvg-convert icon/app_icon.svg | convert -scale 256x256 - icon/dev.tunnel.Tunnel.ico
wine rcedit-x64.exe package/tunnel.exe --set-icon icon/dev.tunnel.Tunnel.ico
```

Both `rsvg-convert` and `rcedit` are available inside the `gtk4-cross-win` Docker image.

---

### 9.7 — CI job for Windows cross-compilation  `[required]`

Add a CI job using Warp's Docker image:

```yaml
windows:
  image: ghcr.io/felinira/gtk4-cross-win:gnome-48
  script:
    - export PKG_CONFIG_PATH=/usr/x86_64-w64-mingw32/sys-root/mingw/lib/pkgconfig/
    - meson setup build -Dprefix="$PWD/package" -Dtarget="x86_64-pc-windows-gnu"
    - ninja -C build && ninja -C build install
    - rsvg-convert data/icons/dev.tunnel.Tunnel.svg | convert -scale 256x256 - tunnel.ico
    - wine rcedit-x64.exe package/tunnel.exe --set-icon tunnel.ico
  artifacts:
    paths:
      - package/
```

The `package/` artifact is the distributable: a ZIP of the directory contains the
`.exe` and all required GTK4/GLib DLLs.

---

### 9.8 — Windows Firewall — user documentation only  `[no code required]`

LocalSend uses UDP multicast on port 53317, which Windows Firewall blocks by default.
On first run, Windows shows the standard "allow network access" prompt. The user clicks
Allow and discovery works normally thereafter. This is the same behaviour as the official
LocalSend Windows client, so users familiar with LocalSend will already know to expect it.

No code required — just a note in the README and/or a first-run dialog.

---

### What Tunnel does NOT need that Warp did

| Warp Windows work | Tunnel status |
|---|---|
| Portal abstraction (`ashpd` gated behind `cfg(unix)`) | Portals not yet added — add as `cfg(unix)` from the start (item 6.1) |
| D-Bus help URI → bundled HTML fallback | No help system planned (item 8.1 is optional) |
| `WINDOWS_BASE_PATH` for resource loading | GResource bundles the assets into the binary — no path lookup needed |
| GStreamer disabled on Windows | No GStreamer in Tunnel |

The net result: Tunnel needs fewer platform-specific `cfg` blocks than Warp did.

---

## Priority order

For a first Flathub-ready release, the strict minimum is:

1. **Protocol adoption** (0.1–0.3) — foundational; all subsequent work assumes LocalSend
2. Cross-platform dependency fixes: `fs2` + `gethostname` (9.1) — do alongside 0.x, no reason to wait
3. Meson build (1.1)
4. AppStream metainfo (2.1)
5. Proper `.desktop` file (2.2) + LocalSend URI scheme (2.5 / 0.4)
6. Symbolic icon (2.3)
7. D-Bus service file (2.4)
8. GResource bundle (2.6)
9. Remove hardcoded colors (3.1)
10. GNotification on incoming + completion (4.1)
11. Flatpak manifest (6.1) + vendor script (6.2)
12. gettext scaffold (5.1) — even English-only

Items 3.2–3.7, 4.x, 7.x, and 8.x can follow in subsequent releases.

The Windows port (section 9) is the final milestone, after the Linux/Flatpak release
is stable. Prerequisites from earlier sections that must land first: protocol adoption
(0.x), Meson build (1.1), GResource bundle (2.6), and the `ashpd` portal work (6.1).
