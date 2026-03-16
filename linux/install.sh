#!/usr/bin/env bash
# Builds and installs Tunnel for the current user.
# Run from the linux/ directory: ./install.sh
# To uninstall: ./install.sh --uninstall

set -euo pipefail

APP_ID="dev.tunnel.Tunnel"
BIN_NAME="tunnel"
INSTALL_BIN="$HOME/.local/bin/$BIN_NAME"
INSTALL_ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
INSTALL_ICON="$INSTALL_ICON_DIR/$APP_ID.png"
INSTALL_SCALABLE_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
INSTALL_SPINNER="$INSTALL_SCALABLE_DIR/search-spinner-symbolic.svg"
INSTALL_DESKTOP="$HOME/.local/share/applications/$APP_ID.desktop"
ICON_SRC="$(dirname "$0")/../icon/app_icon.png"
SPINNER_SRC="$(dirname "$0")/src/assets/hicolor/scalable/apps/search-spinner-symbolic.svg"

RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
RESET='\033[0m'

step() { echo -e "${BOLD}▶ $*${RESET}"; }
ok()   { echo -e "  ${GREEN}✓${RESET}  $*"; }
err()  { echo -e "  ${RED}✗${RESET}  $*"; exit 1; }

# ── Uninstall ──────────────────────────────────────────────────────────────────
if [[ "${1:-}" == "--uninstall" ]]; then
  step "Uninstalling Tunnel…"
  rm -f "$INSTALL_BIN" && ok "Removed binary"
  rm -f "$INSTALL_ICON" && ok "Removed app icon"
  rm -f "$HOME/.local/share/icons/hicolor/scalable/apps/search-spinner-symbolic.svg" && ok "Removed search spinner"
  rm -f "$INSTALL_DESKTOP" && ok "Removed desktop entry"
  update-desktop-database "$HOME/.local/share/applications/" 2>/dev/null || true
  echo -e "\n${BOLD}Tunnel uninstalled.${RESET}"
  exit 0
fi

# ── Verify we're in the right directory ───────────────────────────────────────
if [[ ! -f "Cargo.toml" ]]; then
  err "Run this script from the linux/ directory: ./install.sh"
fi

if [[ ! -f "$ICON_SRC" ]]; then
  err "Icon not found at $ICON_SRC"
fi

# ── Build ──────────────────────────────────────────────────────────────────────
step "Building release binary…"
cargo build --release
ok "Build complete"

# ── Install dirs ───────────────────────────────────────────────────────────────
mkdir -p "$HOME/.local/bin"
mkdir -p "$INSTALL_ICON_DIR"
mkdir -p "$INSTALL_SCALABLE_DIR"
mkdir -p "$HOME/.local/share/applications"

# ── Binary ────────────────────────────────────────────────────────────────────
step "Installing binary…"
cp "target/release/$BIN_NAME" "$INSTALL_BIN"
chmod +x "$INSTALL_BIN"
ok "Installed to $INSTALL_BIN"

# ── Icon ──────────────────────────────────────────────────────────────────────
step "Installing icons…"
cp "$ICON_SRC" "$INSTALL_ICON"
ok "Installed app icon to $INSTALL_ICON"
cp "$SPINNER_SRC" "$INSTALL_SPINNER"
ok "Installed search spinner to $INSTALL_SPINNER"

# Update icon cache so GNOME picks it up
if command -v gtk-update-icon-cache &>/dev/null; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor/" 2>/dev/null || true
fi

# ── Desktop entry ─────────────────────────────────────────────────────────────
step "Creating desktop entry…"
cat > "$INSTALL_DESKTOP" <<EOF
[Desktop Entry]
Name=Tunnel
Comment=Simple and secure P2P file transfer
Exec=$INSTALL_BIN
Icon=$APP_ID
Type=Application
Categories=Network;FileTransfer;
StartupNotify=true
StartupWMClass=tunnel
EOF
ok "Installed to $INSTALL_DESKTOP"

update-desktop-database "$HOME/.local/share/applications/" 2>/dev/null || true

# ── Check PATH ────────────────────────────────────────────────────────────────
if ! echo "$PATH" | grep -q "$HOME/.local/bin"; then
  echo ""
  echo -e "  ${RED}!${RESET}  ~/.local/bin is not in your PATH."
  echo "     Add this to your ~/.bashrc or ~/.zshrc:"
  echo '     export PATH="$HOME/.local/bin:$PATH"'
fi

echo ""
echo -e "${GREEN}${BOLD}Tunnel installed.${RESET} You can now launch it from the GNOME app menu."
echo "  To uninstall: ./install.sh --uninstall"
