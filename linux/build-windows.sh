#!/usr/bin/env bash
# Cross-compiles Tunnel for Windows (x86_64) from Linux.
# Run from the linux/ directory: ./build-windows.sh
#
# Output: build-windows/tunnel.exe
#
# Requirements (first run will tell you what's missing):
#   rustup target add x86_64-pc-windows-gnu
#   sudo apt install gcc-mingw-w64-x86-64   # or equivalent on your distro

set -euo pipefail

TARGET="x86_64-pc-windows-gnu"
BUILD_DIR="build-windows"
OUTPUT="$BUILD_DIR/tunnel.exe"

RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
RESET='\033[0m'

step() { echo -e "${BOLD}▶ $*${RESET}"; }
ok()   { echo -e "  ${GREEN}✓${RESET}  $*"; }
err()  { echo -e "  ${RED}✗${RESET}  $*"; exit 1; }

# ── Verify we're in the right directory ───────────────────────────────────────
if [[ ! -f "Cargo.toml" ]]; then
  err "Run this script from the linux/ directory: ./build-windows.sh"
fi

# ── Check dependencies ─────────────────────────────────────────────────────────
step "Checking dependencies…"

if ! command -v cargo &>/dev/null; then
  err "cargo not found. Install Rust from https://rustup.rs"
fi

if ! rustup target list --installed | grep -q "$TARGET"; then
  echo "  Installing Rust target $TARGET…"
  rustup target add "$TARGET"
fi
ok "Rust target $TARGET is installed"

if ! command -v x86_64-w64-mingw32-gcc &>/dev/null; then
  err "mingw-w64 cross-compiler not found.\n     Install it with: sudo apt install gcc-mingw-w64-x86-64"
fi
ok "mingw-w64 cross-compiler found"

# ── Build ──────────────────────────────────────────────────────────────────────
step "Cross-compiling for $TARGET…"
cargo build --release --target "$TARGET"
ok "Build complete"

# ── Collect output ─────────────────────────────────────────────────────────────
step "Collecting output…"
mkdir -p "$BUILD_DIR"
cp "target/$TARGET/release/tunnel.exe" "$OUTPUT"
ok "Binary: $OUTPUT"

echo ""
echo -e "${GREEN}${BOLD}Done.${RESET} Copy ${BOLD}$OUTPUT${RESET} to your Windows machine and run it."
echo "  GTK4 runtime must be installed on Windows for the app to start."
