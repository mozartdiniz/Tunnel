# Tunnel

A simple, secure peer-to-peer file transfer app for local networks. No cloud, no accounts, no internet required — just devices on the same Wi-Fi or Ethernet.

Tunnel implements the [LocalSend v2 open protocol](https://github.com/localsend/protocol), making it fully interoperable with the ~50 million existing LocalSend users on Android, iOS, Windows, and macOS.

---

## Platforms

| Platform | UI Framework | Status |
|---|---|---|
| Linux | GTK4 + Libadwaita (GNOME) | Stable, Flatpak-ready |
| macOS | SwiftUI | Stable |
| Windows | Rust + GTK4 (cross-compiled) | Beta |

---

## Features

- **Automatic peer discovery** — devices on the same network appear instantly via UDP multicast. Falls back to a subnet scan if multicast is blocked.
- **Send files** — select a peer, pick files or folders, and send. Folders are ZIP-compressed automatically.
- **Receive files** — incoming transfers show an accept/deny dialog. Accepted files land in your Downloads folder (configurable).
- **Multi-file transfers** — send any number of files in a single session.
- **Transfer progress** — real-time speed, bytes transferred, and estimated time remaining.
- **Cancel mid-transfer** — both sender and receiver can cancel at any point.
- **File integrity verification** — SHA-256 checksums computed during transfer; mismatched files are discarded automatically.
- **TOFU security** — first-contact fingerprint pinning detects certificate changes on subsequent connections.
- **Sleep inhibition** — prevents the system from sleeping during active transfers (Linux).
- **Configurable device name and download directory** — via the preferences dialog.

---

## Protocol

Tunnel speaks **LocalSend v2** — an open, documented protocol for LAN file transfer over HTTPS.

### Discovery

Every device advertises itself by sending a JSON payload to the UDP multicast group `224.0.0.167:53317` every 10 seconds. The payload contains:

```json
{
  "alias": "Alice's Laptop",
  "fingerprint": "<sha256-of-tls-cert>",
  "version": "2.0",
  "deviceType": "desktop",
  "port": 53317,
  "protocol": "https",
  "download": true
}
```

Peers are considered gone if no heartbeat is received within 30 seconds. If multicast doesn't surface any peers within 5 seconds (e.g., on managed networks where multicast is filtered), Tunnel falls back to probing every address in the local `/24` subnet over HTTPS.

Tunnel joins the multicast group on **every active non-loopback IPv4 interface** and broadcasts out each interface independently. This ensures discovery works when Ethernet and Wi-Fi are active simultaneously.

### Transfer flow

1. **Prepare** — sender POSTs file metadata (names, sizes, SHA-256 hashes) to the receiver's `/api/localsend/v2/prepare-upload`. The receiver shows an accept/deny dialog to its user.
2. **Tokens** — if accepted, the receiver returns a session ID and a per-file upload token.
3. **Upload** — sender streams each file to `/api/localsend/v2/upload?sessionId=…&fileId=…&token=…`. The receiver hashes the stream in flight and verifies the checksum when the stream ends.
4. **Complete** — receiver atomically renames the temp file to its final destination once the checksum passes. On mismatch, the temp file is deleted.
5. **Cancel** — either party can POST to `/api/localsend/v2/cancel` to abort a session.

### HTTP API (receiver endpoints)

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/localsend/v2/info` | Return device info |
| `POST` | `/api/localsend/v2/prepare-upload` | Initiate transfer, show dialog |
| `POST` | `/api/localsend/v2/upload` | Stream file bytes |
| `POST` | `/api/localsend/v2/cancel` | Abort session |

### Security

- **HTTPS everywhere** — each device generates a self-signed TLS certificate on first run, stored in the app data directory. All transfers are encrypted.
- **TOFU fingerprinting** — the device fingerprint (SHA-256 of the TLS certificate) is announced via UDP. On first contact, Tunnel stores the fingerprint. On subsequent contacts, it warns the user if the fingerprint changed.
- **File integrity** — SHA-256 checksums are verified after every transfer. Corrupted or tampered files are discarded before reaching the Downloads folder.
- **No shared secret** — there is no pairing or authentication. The security gate is the accept/deny dialog shown to the receiver. Any device on the LAN can send a transfer request; the receiver's user decides whether to accept it. This is by design (same as LocalSend).

### Ports and firewall

| Protocol | Port | Purpose |
|---|---|---|
| TCP | 53317 | HTTPS server (transfer + info endpoint) |
| UDP | 53317 | Multicast announcements |

Multicast address: `224.0.0.167`

On Windows, the installer adds a firewall rule for UDP 53317. On macOS, the OS prompts on first run. On Linux with Flatpak, the sandbox permissions handle it automatically.

---

## Building

### Linux

#### System dependencies

```bash
# Debian/Ubuntu
sudo apt-get install \
  meson ninja-build \
  cargo \
  libgtk-4-dev \
  libadwaita-1-dev \
  pkg-config

# Fedora
sudo dnf install \
  meson ninja-build \
  cargo \
  gtk4-devel \
  libadwaita-devel \
  pkg-config
```

#### Option 1 — developer install (quickest)

```bash
cd linux
./install.sh
```

Builds a release binary and installs to `~/.local/bin/tunnel`, along with the `.desktop` file and icon.

#### Option 2 — Meson (system install)

```bash
cd linux
meson setup build
meson compile -C build
sudo meson install -C build
```

#### Option 3 — Flatpak (recommended for distribution)

First, generate the offline cargo sources from the lock file:

```bash
python3 linux/build-aux/flatpak-cargo-generator.py \
  linux/Cargo.lock \
  --module-name tunnel \
  -o linux/build-aux/cargo-sources.json
```

Then build and install locally:

```bash
flatpak-builder --user --install build-dir linux/build-aux/dev.tunnel.Tunnel.yaml
flatpak run dev.tunnel.Tunnel
```

Or use the development profile (adds a blue banner to the title bar):

```bash
flatpak-builder --user --install build-dir linux/build-aux/dev.tunnel.Tunnel.Devel.yaml
flatpak run dev.tunnel.Tunnel.Devel
```

> **Note:** `cargo-sources.json` must be regenerated any time `Cargo.lock` changes. Commit both files together.

#### App data (Linux)

| Path | Contents |
|---|---|
| `~/.local/share/tunnel/tunnel/config.json` | Device name, download directory |
| `~/.local/share/tunnel/tunnel/cert.der` | Self-signed TLS certificate |
| `~/.local/share/tunnel/tunnel/key.der` | Private key |
| `~/.local/share/tunnel/tunnel/known_peers.json` | TOFU fingerprint store |

#### Logging

```bash
RUST_LOG=tunnel=debug tunnel      # normal verbosity
RUST_LOG=tunnel=trace tunnel      # maximum verbosity
```

---

### macOS

#### Requirements

- macOS 13.0 or later
- Xcode 15 or later (Swift 5.9+)

```bash
xcode-select --install
```

#### Development build

```bash
cd mac
./build.sh
```

Builds for the native architecture and installs `Tunnel.app` to `/Applications`.

#### Release build (for distribution)

Requires an [Apple Developer Program](https://developer.apple.com/programs/) membership ($99/year) and a Developer ID Application certificate.

```bash
cd mac
./build.sh --dist \
  --sign "Developer ID Application: Your Name (TEAMID)" \
  --apple-id your@apple.id \
  --team-id ABCDE12345 \
  --password "xxxx-xxxx-xxxx-xxxx" \
  --version 1.0
```

This produces a notarized, stapled `Tunnel-1.0.dmg` ready for distribution. The script handles the full pipeline: universal binary (`arm64` + `x86_64` via `lipo`), app icon generation, code signing with hardened runtime, notarization, and stapling.

---

### Windows

Windows builds are cross-compiled from Linux using MinGW-w64.

#### Requirements

```bash
# Install the Rust Windows target
rustup target add x86_64-pc-windows-gnu

# Install the MinGW cross-compiler (Debian/Ubuntu)
sudo apt-get install gcc-mingw-w64-x86-64
```

#### Build

```bash
cd linux
./build-windows.sh
```

Output: `build-windows/tunnel.exe`

#### Distribution

The distribution target uses [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) to produce a WiX MSI installer. The GTK4 runtime DLLs must be present at `C:\gtk-build\gtk\x64\release\bin\` and will be bundled into the installer. See `linux/packaging/windows/` for the WiX fragment that registers the required firewall rule.

---

## Architecture

Tunnel uses a **two-thread model** to keep the UI responsive at all times.

```
┌───────────────────────────┐   AppCommand   ┌────────────────────────────────────┐
│     GTK Main Thread       │ ─────────────▶ │     Network Thread (Tokio)         │
│                           │                │                                    │
│  - Renders UI             │ ◀───────────── │  - HTTPS server (axum, port 53317) │
│  - Handles user input     │   AppEvent     │  - UDP multicast discovery         │
│  - Updates peer list      │                │  - Outgoing file transfers         │
│  - Shows dialogs          │                │  - Incoming transfer handling      │
└───────────────────────────┘                └────────────────────────────────────┘
```

**AppCommand** (UI → network): `SendFiles`, `AcceptTransfer`, `DenyTransfer`, `CancelTransfer`, `UpdateDeviceName`, `UpdateDownloadDir`

**AppEvent** (network → UI): `PeerDiscovered`, `PeerLost`, `IncomingRequest`, `TransferProgress`, `TransferComplete`, `TransferError`

The two threads communicate over `async_channel` unbounded queues. The GTK thread never performs blocking I/O. The network thread never touches GTK objects.

The network thread runs a Tokio async runtime hosting the axum HTTP router and the UDP multicast broadcaster/listener, all started on a single dedicated OS thread at application startup.

Shared mutable state (active sessions, config, download directory) is protected with `Arc<RwLock<T>>` or `Arc<Mutex<T>>`.

---

## Project structure

```
Tunnel/
├── icon/
│   └── app_icon.png                    # Master icon (all platforms derive from this)
├── linux/                              # Linux + Windows
│   ├── src/
│   │   ├── main.rs                     # Entry: register GResources, run Application
│   │   ├── application.rs              # AdwApplication: startup, network thread, channels
│   │   ├── window/
│   │   │   ├── mod.rs                  # Public Window API
│   │   │   └── imp.rs                  # GObject impl, template children, signal wiring
│   │   ├── app/
│   │   │   ├── mod.rs                  # AppState, AppEvent, AppCommand
│   │   │   ├── network.rs              # Tokio runtime, axum router, run_network()
│   │   │   ├── actions.rs              # User-initiated actions (send, cancel…)
│   │   │   ├── state.rs                # Transfer session state
│   │   │   ├── types.rs                # Protocol data types (DeviceInfo, FileMetadata)
│   │   │   └── handlers/
│   │   │       ├── info.rs             # GET  /api/localsend/v2/info
│   │   │       ├── prepare_upload.rs   # POST /api/localsend/v2/prepare-upload
│   │   │       ├── upload.rs           # POST /api/localsend/v2/upload
│   │   │       └── cancel.rs           # POST /api/localsend/v2/cancel
│   │   ├── ui/                         # Stateless UI helpers (peer list, dialogs, prefs)
│   │   ├── discovery.rs                # UDP multicast + subnet scan fallback
│   │   ├── localsend.rs                # Protocol constants and helpers
│   │   ├── tls/                        # Cert generation, TOFU verifier, TLS stack setup
│   │   ├── transfer/                   # File streaming, ZIP, integrity check
│   │   ├── config.rs                   # Config struct, load/save
│   │   └── inhibit.rs                  # Sleep inhibit via D-Bus (Linux only)
│   ├── data/
│   │   ├── dev.tunnel.Tunnel.desktop.in        # Desktop entry
│   │   ├── dev.tunnel.Tunnel.metainfo.xml.in   # AppStream metadata
│   │   ├── icons/                              # Hicolor icon set
│   │   └── resources/                          # GResource bundle (UI XML, CSS, icons)
│   ├── build-aux/
│   │   ├── dev.tunnel.Tunnel.yaml              # Flatpak release manifest
│   │   ├── dev.tunnel.Tunnel.Devel.yaml        # Flatpak development manifest
│   │   ├── flatpak-cargo-generator.py          # Generates cargo-sources.json
│   │   └── cargo-sources.json                  # Offline crate list for Flatpak builds
│   ├── packaging/windows/
│   │   └── firewall.wxs                        # WiX fragment: UDP firewall rule
│   ├── Cargo.toml
│   ├── Cargo.lock
│   └── meson.build
├── mac/                                # macOS
│   ├── Sources/Tunnel/
│   │   ├── TunnelApp.swift             # SwiftUI entry point
│   │   ├── AppModel.swift              # State: peers, transfers, settings
│   │   ├── ContentView.swift           # Main UI
│   │   ├── Discovery.swift             # UDP multicast peer discovery
│   │   ├── Protocol.swift              # LocalSend v2 data types
│   │   ├── Transfer.swift              # Outgoing file sending
│   │   ├── TLSManager.swift            # Cert generation, TOFU verification
│   │   └── Config.swift                # Settings persistence
│   ├── Package.swift
│   └── build.sh
└── .github/
    └── workflows/
        └── flatpak-release.yml         # CI: Flatpak bundle artifact on every push to main
```

---

## Interoperability

Tunnel is interoperable with any app that implements LocalSend v2:

- [LocalSend](https://localsend.org) — Android, iOS, Windows, macOS, Linux

If a LocalSend device is on your network, it will appear in Tunnel's peer list automatically.

---

## License

[MIT](LICENSE) — Mozart Diniz
