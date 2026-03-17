#!/usr/bin/env bash
# Vendor all Cargo dependencies for offline Flatpak builds.
# Usage: ./build-aux/dist-vendor.sh <source-root>
#
# Produces a .cargo/config.toml that redirects cargo to the vendored sources,
# and a cargo-sources.json that the Flatpak manifest can consume via
# flatpak-cargo-generator.py.

set -euo pipefail

SOURCE_ROOT="${1:-$(dirname "$0")/..}"

cd "$SOURCE_ROOT"

echo "Vendoring dependencies…"
mkdir -p .cargo
cargo vendor vendor 2>&1 | tee .cargo/config.toml.tmp

cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

echo "Done. Vendored sources in ./vendor"
echo "Run flatpak-cargo-generator.py to generate cargo-sources.json for the Flatpak manifest."
