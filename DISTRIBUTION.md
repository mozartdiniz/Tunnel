# Distribution Readiness Checklist

Reviewed 2026-03-19.

---

## Linux — Flatpak (~75% ready)

### Blocking (must fix before Flathub submission)

- [ ] Add `LICENSE` file to repo root with GPL-3.0-or-later text
- [ ] Fix placeholder author in `linux/Cargo.toml`: `authors = ["Your Name <email@example.com>"]`
- [ ] Add `license = "GPL-3.0-or-later"` field to `linux/Cargo.toml`
- [ ] Add screenshots to `linux/data/dev.tunnel.Tunnel.metainfo.xml.in` — Flathub requires at least 1. Take 2-3 screenshots, host on GitHub (raw.githubusercontent.com), add a `<screenshots>` block with `<image>` URLs and `<caption>` text

### Strongly recommended

- [ ] Generate PNG icons at 256×256, 128×128, 64×64, 48×48 — only a scalable SVG exists at `linux/data/icons/hicolor/scalable/apps/dev.tunnel.Tunnel.svg`
- [ ] Add `repository = "https://github.com/mozartdiniz/Tunnel"` to `linux/Cargo.toml`
- [ ] Expand README.md (currently nearly empty)

### Already good — no action needed

- Flatpak manifests in `build-aux/*.yaml` — correct permissions, offline Cargo build configured
- Desktop file — valid and complete
- Metainfo structure — complete except screenshots
- Meson build files — correct
- D-Bus service file — correct
- Version consistency — all files at 0.1.0
- No hardcoded paths, no leftover debug `println!`s

---

## Mac — Direct website download (~90% ready)

The `mac/build.sh` script already handles everything: universal binary (arm64 + x86_64), code signing, notarization, and DMG creation. The only blockers are credentials.

> **Important:** Apple requires notarization for all apps distributed outside the App Store since macOS Catalina. Without it, Gatekeeper shows a scary warning on every user's Mac. You cannot skip this.

### Blocking

- [ ] Enroll in Apple Developer Program ($99/year) at developer.apple.com — required to get a Developer ID Application certificate
- [ ] Generate an app-specific password at appleid.apple.com (separate from your regular Apple ID password)
- [ ] Once you have the cert, build and notarize with:
  ```bash
  cd mac
  ./build.sh --dist \
    --sign "Developer ID Application: Your Name (TEAMID)" \
    --apple-id your@apple.id \
    --team-id ABCDE12345 \
    --password xxxx-xxxx-xxxx-xxxx \
    --version 1.0
  # Output: .build/dist/Tunnel-1.0.dmg — notarized, stapled, ready to host
  ```

### Optional improvements

- [ ] Add Sparkle auto-update framework so users get notified of new versions
- [ ] Add download/install instructions to README

### Already good — no action needed

- Universal binary via lipo ✓
- Hardened Runtime entitlements ✓
- Code signing with `--options runtime --timestamp` ✓
- Full notarization automation in build.sh ✓
- DMG creation and signing ✓
- Icon generation for all required macOS sizes ✓
- Info.plist with correct metadata ✓
- No App Store-only APIs ✓
- macOS 13.0 minimum deployment target ✓

---

## Windows — MSI installer (~40% ready)

The code is cross-platform ready and build scripts exist, but several things are needed before public distribution.

### Blocking

- [ ] **Generate a `.ico` file** — no Windows icon exists. Convert `icon/app_icon.png` to `.ico` with sizes 16, 32, 48, 256px (use ImageMagick, GIMP, or an online converter). Reference it in `linux/Cargo.toml` under `[package.metadata.packager]`
- [ ] **Bundle GTK4 runtime in the MSI** — current scripts assume GTK4 pre-installed at `C:\gtk-build\gtk\x64\release\`. End users won't have this. Include all required DLLs in the installer. Use the `gtk4-cross-win` Docker image (see `ROADMAP.md` item 9.7) to produce a self-contained build
- [ ] **Fix `RevealFile` on Windows** — `linux/src/application.rs` lines 130–142 use `gio::AppInfo::launch_default_for_uri()` which doesn't work on Windows. Add a `#[cfg(target_os = "windows")]` variant:
  ```rust
  #[cfg(target_os = "windows")]
  let _ = std::process::Command::new("explorer")
      .arg("/select,")
      .arg(&path_str)
      .spawn();
  ```
- [ ] **Code signing certificate** — unsigned MSI/EXE will be flagged by Windows SmartScreen on every download. Acquire a standard code signing cert (~$100/yr) or EV cert (~$300/yr) and integrate a signing step into `build-windows.ps1`

### Strongly recommended

- [ ] Add a Windows build job to `.github/workflows/ci.yml`
- [ ] Document Windows prerequisites in README: Windows 10+, firewall rule for UDP port 53317
- [ ] Verify the cargo-packager WiX output creates a Start Menu shortcut and a Control Panel uninstall entry
- [ ] Test full install/uninstall cycle on a clean Windows VM

### Already good — no action needed

- `windows_subsystem = "windows"` configured in `main.rs` ✓
- `windows_hacks()` for console reattachment ✓
- `[package.metadata.packager]` with `formats = ["wix", ...]` in Cargo.toml ✓
- `build-windows.ps1` and `build-windows.sh` scripts exist ✓
- Cross-platform dependencies (gethostname, fs2) already integrated ✓
