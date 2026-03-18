#!/bin/bash
# ─────────────────────────────────────────────────────────────────────────────
# build.sh — Tunnel macOS build & package script
#
#   ./build.sh
#       Debug build → installs Tunnel.app to /Applications (dev workflow).
#
#   ./build.sh --dist --sign "Developer ID Application: Your Name (TEAMID)"
#       Release universal binary → signed .app → DMG in .build/dist/.
#       Notarization is performed automatically when Apple ID credentials
#       are supplied (recommended before posting the DMG for download).
#
# Notarization flags (all required together, or omit all three to skip):
#   --apple-id  your@apple.id
#   --team-id   ABCDE12345
#   --password  xxxx-xxxx-xxxx-xxxx   (app-specific password from appleid.apple.com)
#
# Version flag (defaults to 1.0):
#   --version   2.0
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

APP_NAME="Tunnel"
BUNDLE_ID="dev.tunnel.app"
VERSION="1.0"
ICON_SRC="../icon/app_icon.png"

DIST=false
SIGN_IDENTITY=""
APPLE_ID=""
TEAM_ID=""
APP_PASSWORD=""

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case $1 in
        --dist)       DIST=true;              shift   ;;
        --sign)       SIGN_IDENTITY="$2";     shift 2 ;;
        --apple-id)   APPLE_ID="$2";          shift 2 ;;
        --team-id)    TEAM_ID="$2";           shift 2 ;;
        --password)   APP_PASSWORD="$2";      shift 2 ;;
        --version)    VERSION="$2";           shift 2 ;;
        *) echo "Unknown flag: $1"; exit 1 ;;
    esac
done

# ── Validate ──────────────────────────────────────────────────────────────────
if $DIST && [[ -z "$SIGN_IDENTITY" ]]; then
    echo "error: --dist requires --sign \"Developer ID Application: Your Name (TEAMID)\""
    exit 1
fi

NOTARIZE=false
if [[ -n "$APPLE_ID" && -n "$TEAM_ID" && -n "$APP_PASSWORD" ]]; then
    NOTARIZE=true
elif [[ -n "$APPLE_ID" || -n "$TEAM_ID" || -n "$APP_PASSWORD" ]]; then
    echo "error: notarization requires all three: --apple-id, --team-id, --password"
    exit 1
fi

# ── Build ─────────────────────────────────────────────────────────────────────
if $DIST; then
    echo "▶ Building universal release binary (arm64 + x86_64)..."
    swift build -c release --arch arm64
    swift build -c release --arch x86_64

    BINARY_DIR=".build/release-universal"
    mkdir -p "$BINARY_DIR"
    lipo -create -output "$BINARY_DIR/$APP_NAME" \
        ".build/arm64-apple-macosx/release/$APP_NAME" \
        ".build/x86_64-apple-macosx/release/$APP_NAME"
    echo "  lipo: $(lipo -info "$BINARY_DIR/$APP_NAME")"
else
    echo "▶ Building debug binary..."
    swift build 2>&1 | grep -v "^$"
    BINARY_DIR=".build/debug"
fi

# ── App icon ──────────────────────────────────────────────────────────────────
echo "▶ Generating app icon..."
if [[ ! -f "$ICON_SRC" ]]; then
    echo "  warning: icon not found at $ICON_SRC — skipping icon"
    SKIP_ICON=true
else
    SKIP_ICON=false
    ICONSET_DIR=$(mktemp -d)/AppIcon.iconset
    mkdir -p "$ICONSET_DIR"

    sips -z 16   16   "$ICON_SRC" --out "$ICONSET_DIR/icon_16x16.png"      >/dev/null
    sips -z 32   32   "$ICON_SRC" --out "$ICONSET_DIR/icon_16x16@2x.png"   >/dev/null
    sips -z 32   32   "$ICON_SRC" --out "$ICONSET_DIR/icon_32x32.png"       >/dev/null
    sips -z 64   64   "$ICON_SRC" --out "$ICONSET_DIR/icon_32x32@2x.png"   >/dev/null
    sips -z 128  128  "$ICON_SRC" --out "$ICONSET_DIR/icon_128x128.png"     >/dev/null
    sips -z 256  256  "$ICON_SRC" --out "$ICONSET_DIR/icon_128x128@2x.png" >/dev/null
    sips -z 256  256  "$ICON_SRC" --out "$ICONSET_DIR/icon_256x256.png"     >/dev/null
    sips -z 512  512  "$ICON_SRC" --out "$ICONSET_DIR/icon_256x256@2x.png" >/dev/null
    sips -z 512  512  "$ICON_SRC" --out "$ICONSET_DIR/icon_512x512.png"     >/dev/null
    cp   "$ICON_SRC"                    "$ICONSET_DIR/icon_512x512@2x.png"

    iconutil -c icns "$ICONSET_DIR" -o "$BINARY_DIR/AppIcon.icns"
    rm -rf "$(dirname "$ICONSET_DIR")"
fi

# ── App bundle ────────────────────────────────────────────────────────────────
APP_BUNDLE="${APP_NAME}.app"
BUNDLE_PATH="$BINARY_DIR/$APP_BUNDLE"

echo "▶ Assembling ${APP_BUNDLE}..."
rm -rf "$BUNDLE_PATH"
mkdir -p "$BUNDLE_PATH/Contents/MacOS"
mkdir -p "$BUNDLE_PATH/Contents/Resources"

cp "$BINARY_DIR/$APP_NAME" "$BUNDLE_PATH/Contents/MacOS/$APP_NAME"

if ! $SKIP_ICON; then
    cp "$BINARY_DIR/AppIcon.icns" "$BUNDLE_PATH/Contents/Resources/AppIcon.icns"
fi

cat > "$BUNDLE_PATH/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>          <string>${BUNDLE_ID}</string>
  <key>CFBundleName</key>                <string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key>         <string>${APP_NAME}</string>
  <key>CFBundleExecutable</key>          <string>${APP_NAME}</string>
  <key>CFBundleIconFile</key>            <string>AppIcon</string>
  <key>CFBundleIconName</key>            <string>AppIcon</string>
  <key>CFBundlePackageType</key>         <string>APPL</string>
  <key>CFBundleShortVersionString</key>  <string>${VERSION}</string>
  <key>CFBundleVersion</key>             <string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key>      <string>13.0</string>
  <key>NSPrincipalClass</key>            <string>NSApplication</string>
  <key>NSHighResolutionCapable</key>     <true/>
  <key>NSSupportsAutomaticTermination</key> <false/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>Tunnel uses the local network to discover and exchange files with nearby devices.</string>
</dict>
</plist>
PLIST

# ── Code signing ──────────────────────────────────────────────────────────────
if $DIST; then
    echo "▶ Signing with hardened runtime..."
    codesign --force --options runtime \
        --entitlements Tunnel.entitlements \
        --sign "$SIGN_IDENTITY" \
        --timestamp \
        "$BUNDLE_PATH"

    echo "▶ Verifying signature..."
    codesign --verify --deep --strict --verbose=2 "$BUNDLE_PATH"
    spctl --assess --type execute --verbose "$BUNDLE_PATH" \
        && echo "  Gatekeeper: OK"
fi

# ── Notarization ──────────────────────────────────────────────────────────────
if $DIST && $NOTARIZE; then
    ZIP_PATH="$BINARY_DIR/${APP_NAME}.zip"

    echo "▶ Notarizing app bundle..."
    ditto -c -k --keepParent "$BUNDLE_PATH" "$ZIP_PATH"
    xcrun notarytool submit "$ZIP_PATH" \
        --apple-id  "$APPLE_ID"    \
        --team-id   "$TEAM_ID"     \
        --password  "$APP_PASSWORD" \
        --wait
    rm "$ZIP_PATH"

    echo "▶ Stapling notarization ticket..."
    xcrun stapler staple "$BUNDLE_PATH"
fi

# ── DMG ───────────────────────────────────────────────────────────────────────
if $DIST; then
    DIST_DIR=".build/dist"
    mkdir -p "$DIST_DIR"
    DMG_PATH="$DIST_DIR/${APP_NAME}-${VERSION}.dmg"

    echo "▶ Creating DMG..."
    rm -f "$DMG_PATH"
    hdiutil create \
        -volname  "$APP_NAME"  \
        -srcfolder "$BUNDLE_PATH" \
        -ov -format UDZO \
        "$DMG_PATH"

    # Sign the DMG itself so Gatekeeper accepts it directly.
    codesign --sign "$SIGN_IDENTITY" --timestamp "$DMG_PATH"

    if $NOTARIZE; then
        echo "▶ Notarizing DMG..."
        xcrun notarytool submit "$DMG_PATH" \
            --apple-id  "$APPLE_ID"    \
            --team-id   "$TEAM_ID"     \
            --password  "$APP_PASSWORD" \
            --wait
        xcrun stapler staple "$DMG_PATH"
    fi

    echo ""
    echo "✓ Distribution package ready:"
    echo "  $DMG_PATH"
    if ! $NOTARIZE; then
        echo ""
        echo "  Note: notarization was skipped."
        echo "  Without notarization, macOS Gatekeeper will block the app on"
        echo "  other Macs. Supply --apple-id, --team-id, and --password to notarize."
    fi
else
    # ── Local install (dev workflow) ──────────────────────────────────────────
    echo "▶ Installing to /Applications..."
    rm -rf "/Applications/$APP_BUNDLE"
    cp -R "$BUNDLE_PATH" "/Applications/"
    echo "✓ Done — /Applications/$APP_BUNDLE"
fi
