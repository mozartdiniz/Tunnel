#!/bin/bash
set -e

APP_NAME="Tunnel"
BUNDLE_ID="dev.tunnel.app"
VERSION="1.0"
ICON_SRC="../icon/app_icon.png"
BUILD_DIR=".build/release"
APP_BUNDLE="${APP_NAME}.app"
INSTALL_DIR="/Applications"

echo "▶ Building release binary..."
swift build -c release 2>&1 | grep -v "^$"

# ── Icon ──────────────────────────────────────────────────────────────────────
echo "▶ Generating app icon..."
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
cp "$ICON_SRC"                     "$ICONSET_DIR/icon_512x512@2x.png"

iconutil -c icns "$ICONSET_DIR" -o "$BUILD_DIR/AppIcon.icns"
rm -rf "$(dirname $ICONSET_DIR)"

# ── App bundle ────────────────────────────────────────────────────────────────
echo "▶ Assembling ${APP_BUNDLE}..."
rm -rf "$BUILD_DIR/$APP_BUNDLE"
mkdir -p "$BUILD_DIR/$APP_BUNDLE/Contents/MacOS"
mkdir -p "$BUILD_DIR/$APP_BUNDLE/Contents/Resources"

cp "$BUILD_DIR/$APP_NAME"          "$BUILD_DIR/$APP_BUNDLE/Contents/MacOS/$APP_NAME"
cp "$BUILD_DIR/AppIcon.icns"       "$BUILD_DIR/$APP_BUNDLE/Contents/Resources/AppIcon.icns"

cat > "$BUILD_DIR/$APP_BUNDLE/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>       <string>${BUNDLE_ID}</string>
  <key>CFBundleName</key>             <string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key>      <string>${APP_NAME}</string>
  <key>CFBundleExecutable</key>       <string>${APP_NAME}</string>
  <key>CFBundleIconFile</key>         <string>AppIcon</string>
  <key>CFBundleIconName</key>         <string>AppIcon</string>
  <key>CFBundlePackageType</key>      <string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key>          <string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key>   <string>13.0</string>
  <key>NSPrincipalClass</key>         <string>NSApplication</string>
  <key>NSHighResolutionCapable</key>  <true/>
  <key>NSSupportsAutomaticTermination</key> <false/>
</dict>
</plist>
PLIST

# ── Install ───────────────────────────────────────────────────────────────────
echo "▶ Installing to ${INSTALL_DIR}..."
rm -rf "${INSTALL_DIR}/${APP_BUNDLE}"
cp -R "$BUILD_DIR/$APP_BUNDLE" "${INSTALL_DIR}/"

echo "✓ Done — ${INSTALL_DIR}/${APP_BUNDLE}"
