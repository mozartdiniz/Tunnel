# Stop on error
$ErrorActionPreference = "Stop"

$TARGET = "x86_64-pc-windows-msvc"
$BUILD_DIR = "build-windows"
$OUTPUT = "$BUILD_DIR\tunnel.exe"
$GTK_ROOT = "C:\gtk-build\gtk\x64\release"

# --- CRITICAL PATHS ---
$env:PKG_CONFIG_PATH = "$GTK_ROOT\lib\pkgconfig"
$env:LIB = "$GTK_ROOT\lib;$env:LIB"
$env:INCLUDE = "$GTK_ROOT\include;$env:INCLUDE"
$env:Path = "$GTK_ROOT\bin;$env:Path"

# Helper functions
function Write-Step { Write-Host "`n>> $($args[0])" -ForegroundColor Cyan }
function Write-Ok   { Write-Host "   OK: $($args[0])" -ForegroundColor Green }
function Write-Err  { Write-Host "   ERROR: $($args[0])" -ForegroundColor Red; exit 1 }

# 1. Verify directory
if (!(Test-Path "Cargo.toml")) {
    Write-Err "Run this script from the directory where Cargo.toml exists."
}

# 2. Check dependencies
Write-Step "Checking dependencies..."
$installedTargets = rustup target list --installed
if ($installedTargets -notmatch $TARGET) {
    Write-Host "   Installing Rust target $TARGET..."
    rustup target add $TARGET
}
# Check if cargo-packager is installed
if (!(Get-Command "cargo-packager" -ErrorAction SilentlyContinue)) {
    Write-Host "   Installing cargo-packager..." -ForegroundColor Yellow
    cargo install cargo-packager --locked
}
Write-Ok "Dependencies are ready."

# 3. Build
Write-Step "Compiling for $TARGET..."
cargo build --release --target $TARGET
Write-Ok "Build complete."

# 4. Prepare target/release for cargo-packager
# This is the "bridge" that fixes the LGHT0103 error
Write-Step "Preparing files for Packager..."
if (!(Test-Path "target\release")) { New-Item -ItemType Directory "target\release" | Out-Null }
Copy-Item "target\$TARGET\release\tunnel.exe" -Destination "target\release\tunnel.exe" -Force
Write-Ok "Files staged in target\release"

# 5. Packager (MSI Generation)
Write-Step "Generating MSI Installer..."
# Add WiX to path just for this session if needed
if (Test-Path "C:\Program Files (x86)\WiX Toolset v3.11\bin") {
    $env:Path += ";C:\Program Files (x86)\WiX Toolset v3.11\bin"
}
cargo packager --release
Write-Ok "Installer generated!"

# 6. Legacy build-windows folder (Optional, keeping for your local tests)
Write-Step "Updating local build-windows folder..."
if (!(Test-Path $BUILD_DIR)) { New-Item -ItemType Directory -Path $BUILD_DIR | Out-Null }
Copy-Item "target\release\tunnel.exe" -Destination $OUTPUT -Force
Copy-Item "$GTK_ROOT\bin\*.dll" -Destination $BUILD_DIR -Force

Write-Host "`nSuccess!" -ForegroundColor Green
Write-Host "1. Portable version: .\$BUILD_DIR\tunnel.exe"
Write-Host "2. MSI Installer: target\release\tunnel_0.1.0_x64_en-US.msi" -ForegroundColor Yellow