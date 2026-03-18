#!/usr/bin/env bash
# Generate build-aux/cargo-sources.json for offline Flatpak builds.
#
# Reads Cargo.lock and produces the JSON source list that flatpak-builder
# uses to pre-download every crate archive before the sandboxed build.
#
# Run this after any change to Cargo.lock (e.g. after `cargo update` or
# adding/removing dependencies), then commit the updated cargo-sources.json.
#
# Usage: ./build-aux/dist-vendor.sh [path/to/source-root]

set -euo pipefail

SOURCE_ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOCKFILE="$SOURCE_ROOT/Cargo.lock"
OUTPUT="$SCRIPT_DIR/cargo-sources.json"

if [[ ! -f "$LOCKFILE" ]]; then
  echo "error: Cargo.lock not found at $LOCKFILE" >&2
  exit 1
fi

echo "Generating $OUTPUT from $LOCKFILE …"
python3 "$SCRIPT_DIR/flatpak-cargo-generator.py" \
  "$LOCKFILE" \
  --module-name tunnel \
  -o "$OUTPUT"

echo ""
echo "Next steps:"
echo "  1. Commit build-aux/cargo-sources.json to the repository."
echo "  2. Re-run this script whenever Cargo.lock changes."
