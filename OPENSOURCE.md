# Open Source Readiness Checklist

Reviewed 2026-03-19. Overall: ~75% ready.

---

## Critical Blockers (must fix before making the repo public)

- [ ] **Add LICENSE file** to repo root with GPL-3.0-or-later text (https://www.gnu.org/licenses/gpl-3.0.txt)
- [ ] **Fix `linux/Cargo.toml`** — replace placeholder author and add missing fields:
  ```toml
  authors = ["Real Name <your@email.com>"]
  license = "GPL-3.0-or-later"
  repository = "https://github.com/mozartdiniz/Tunnel"
  homepage = "https://github.com/mozartdiniz/Tunnel"
  ```
- [ ] **Write a real README.md** — currently just `# Tunnel` (1 line). Needs: description, screenshots, build instructions for Linux/Mac/Windows, and basic usage guide
- [ ] **Fix security vulnerabilities documented in `SECURITY_AUDIT.md`** — these are real attack vectors that will get flagged immediately when the project goes public:
  - **Sender impersonation:** receiver never verifies sender certificate — attacker can impersonate any device
  - **No file size limits:** DoS via unbounded upload
  - **Path traversal:** `..` and `.` not rejected as filenames
  - **Checksum verified after write:** file written to disk before integrity is confirmed — should checksum before writing or roll back on failure
- [ ] **Add screenshots** to `linux/data/dev.tunnel.Tunnel.metainfo.xml.in` (also needed for Flathub — see DISTRIBUTION.md)

---

## High Priority (do before announcing the project)

- [ ] **Create `CONTRIBUTING.md`** — how to report bugs, how to submit PRs, coding standards
- [ ] **Create `CODE_OF_CONDUCT.md`** — expected in any public open source project (use the Contributor Covenant: https://www.contributor-covenant.org/)
- [ ] **Expand `.gitignore`** — currently very minimal. Add:
  ```
  .DS_Store
  *.swp
  *.swo
  __pycache__/
  *.pyc
  .vscode/
  .idea/
  .claude/
  ```
  Note: `.claude/settings.local.json` is currently committed — remove it and add `.claude/` to `.gitignore`

---

## Medium Priority (housekeeping)

- [ ] Move or remove files that shouldn't be at the repo root:
  - `audit.py` — security testing script
  - `spoof_attack.py` — attack simulation script
  - `traversal_test.log`, `tunnel_test.log`, `tunnel.log` — test logs
  - Consider moving to a `scripts/` or `tests/` directory, or `.gitignore` them

---

## Already good — no action needed

- Cargo.lock committed ✓ (correct for binary crates)
- Meson build system correctly configured ✓
- AppStream metainfo complete (except screenshots) ✓
- Desktop file valid ✓
- D-Bus service file correct ✓
- CI workflow validates and builds Flatpak ✓
- Flatpak manifests (stable + devel) configured ✓
- No hardcoded secrets, API keys, or tokens in source ✓
- TLS implementation uses audited libraries (rustls + ring) ✓
- ROADMAP.md is detailed and appropriate for public ✓
- i18n infrastructure in place ✓
- Cross-platform code properly gated with `#[cfg(...)]` ✓
