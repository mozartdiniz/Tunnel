#!/usr/bin/env bash
# Install Tunnel on Steam Deck (SteamOS 3.x / Arch-based).
# Run from the linux/ directory: ./install-steamdeck.sh
#
# NOTE: SteamOS has a read-only root filesystem. System packages installed
# here (gtk4, libadwaita, etc.) are wiped on every OS update. Re-run this
# script after updating SteamOS to restore them.
# The Rust toolchain and the Tunnel binary live in your home directory and
# survive OS updates.

set -euo pipefail

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
RESET='\033[0m'

step() { echo -e "\n${BOLD}▶ $*${RESET}"; }
ok()   { echo -e "  ${GREEN}✓${RESET}  $*"; }
warn() { echo -e "  ${YELLOW}!${RESET}  $*"; }
err()  { echo -e "  ${RED}✗${RESET}  $*" >&2; exit 1; }

# ── Sanity checks ─────────────────────────────────────────────────────────────
[[ -f "Cargo.toml" ]] || err "Run this script from the linux/ directory: ./install-steamdeck.sh"
[[ -f "install.sh" ]] || err "install.sh not found in the linux/ directory"

echo -e "${BOLD}Tunnel – Steam Deck installer${RESET}"
echo ""
warn "System packages (gtk4, libadwaita, …) will be wiped on every SteamOS update."
warn "Re-run this script after updating SteamOS to restore them."
echo ""
read -rp "Continue? [y/N] " _yn
[[ "$_yn" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 0; }

# ── 1. Unlock root filesystem ─────────────────────────────────────────────────
step "Disabling read-only filesystem…"
if command -v steamos-readonly &>/dev/null; then
    sudo steamos-readonly disable
    ok "Root filesystem is now writable"
else
    warn "steamos-readonly not found — skipping (not SteamOS?)"
fi

# ── 2. Pacman keyring ─────────────────────────────────────────────────────────
step "Initialising pacman keyring…"
sudo pacman-key --init
sudo pacman-key --populate archlinux
# SteamOS uses its own 'holo' keyring on top of Arch's.
sudo pacman-key --populate holo 2>/dev/null || true
ok "Keyring ready"

# ── 3. System dependencies ────────────────────────────────────────────────────
step "Installing system packages…"
sudo pacman -Sy --needed --noconfirm \
    base-devel \
    gtk4 \
    libadwaita \
    glib2 \
    librsvg \
    dbus
ok "System packages installed"

# ── 4. Re-lock root filesystem ────────────────────────────────────────────────
step "Re-enabling read-only filesystem…"
if command -v steamos-readonly &>/dev/null; then
    sudo steamos-readonly enable
    ok "Root filesystem is read-only again"
fi

# ── 5. Rust toolchain ─────────────────────────────────────────────────────────
step "Checking Rust toolchain…"
# Ensure cargo is on PATH for this session even if already installed.
export PATH="$HOME/.cargo/bin:$PATH"

if command -v cargo &>/dev/null; then
    ok "Rust already installed — $(cargo --version)"
    rustup update stable --no-self-update 2>/dev/null || true
else
    echo "  Downloading and installing rustup…"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
    ok "Rust installed — $(cargo --version)"
fi

# ── 6. Build and install Tunnel ───────────────────────────────────────────────
step "Building and installing Tunnel…"
bash ./install.sh

# ── 7. PATH reminder ──────────────────────────────────────────────────────────
CARGO_BIN="$HOME/.cargo/bin"
LOCAL_BIN="$HOME/.local/bin"

_missing_paths=()
echo "$PATH" | tr ':' '\n' | grep -qxF "$CARGO_BIN" || _missing_paths+=("$CARGO_BIN")
echo "$PATH" | tr ':' '\n' | grep -qxF "$LOCAL_BIN"  || _missing_paths+=("$LOCAL_BIN")

if [[ ${#_missing_paths[@]} -gt 0 ]]; then
    echo ""
    warn "Add the following to ~/.bashrc (or ~/.bash_profile) so the app"
    warn "and Rust tools are available in future terminal sessions:"
    echo ""
    for p in "${_missing_paths[@]}"; do
        echo "    export PATH=\"$p:\$PATH\""
    done
fi

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}${BOLD}Done!${RESET} Tunnel is installed."
echo "  Launch it from the KDE application menu, or run: tunnel"
echo "  To uninstall: ./install.sh --uninstall"
echo ""
warn "Remember: system packages are wiped on SteamOS updates."
warn "Re-run ./install-steamdeck.sh after any OS update."
