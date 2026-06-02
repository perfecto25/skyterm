#!/usr/bin/env bash
# Must be run on macOS. Produces Skyterm.app + Skyterm.dmg.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[+]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
die()   { echo -e "${RED}[✗]${NC} $*" >&2; exit 1; }

APP_NAME="Skyterm"
BUNDLE_ID="com.skyterm.app"
BINARY_NAME="skyterm"
VERSION=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json; pkgs=json.load(sys.stdin)['packages']; \
      print(next(p['version'] for p in pkgs if p['name']=='skyterm-gui'))")

APP_DIR="target/macos/${APP_NAME}.app"
CONTENTS="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS}/MacOS"
RESOURCES_DIR="${CONTENTS}/Resources"
DMG_OUT="target/macos/${APP_NAME}-${VERSION}.dmg"

# ── Prerequisites ────────────────────────────────────────────────────────────

check_deps() {
    info "Checking prerequisites..."

    [[ "$(uname)" == "Darwin" ]] || die "This script must run on macOS"

    command -v cargo         &>/dev/null || die "cargo not found"
    command -v brew          &>/dev/null || die "Homebrew not found — install from https://brew.sh"

    if ! command -v dylibbundler &>/dev/null; then
        warn "dylibbundler not found — installing via Homebrew..."
        brew install dylibbundler
    fi

    if ! command -v create-dmg &>/dev/null; then
        warn "create-dmg not found — installing via Homebrew..."
        brew install create-dmg
    fi

    info "Prerequisites OK"
}

# ── Build ────────────────────────────────────────────────────────────────────

build_release() {
    info "Building release binary..."

    cargo build \
        --release \
        --package skyterm-gui \
        --locked

    [[ -f "target/release/${BINARY_NAME}" ]] \
        || die "Binary not found at target/release/${BINARY_NAME}"

    info "Binary built ($(du -sh "target/release/${BINARY_NAME}" | cut -f1))"
}

# ── .app bundle ──────────────────────────────────────────────────────────────

make_icon() {
    # Build an .icns from the SVG (requires rsvg-convert or Inkscape) or fall
    # back to the small PNG that ships in resources/.
    local png_src="skyterm-gui/resources/skyterm_sm.png"
    local iconset="target/macos/${APP_NAME}.iconset"

    mkdir -p "$iconset"

    if command -v rsvg-convert &>/dev/null; then
        info "Rasterising SVG icon with rsvg-convert..."
        rsvg-convert -w 1024 -h 1024 skyterm-gui/resources/skyterm.svg \
            -o target/macos/icon_1024.png
        png_src="target/macos/icon_1024.png"
    else
        warn "rsvg-convert not found (brew install librsvg); using small fallback PNG"
        warn "Icon quality may be poor — provide a 512×512+ PNG for best results"
    fi

    # macOS requires all these sizes in the iconset
    for size in 16 32 64 128 256 512; do
        sips -z $size $size "$png_src" \
            --out "${iconset}/icon_${size}x${size}.png"      &>/dev/null
        sips -z $((size*2)) $((size*2)) "$png_src" \
            --out "${iconset}/icon_${size}x${size}@2x.png"   &>/dev/null
    done

    iconutil -c icns "$iconset" -o "${RESOURCES_DIR}/${APP_NAME}.icns"
    info "Icon created"
}

make_app_bundle() {
    info "Creating .app bundle..."

    rm -rf "$APP_DIR"
    mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

    # Info.plist
    cat > "${CONTENTS}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
    "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>      <string>${BUNDLE_ID}</string>
    <key>CFBundleName</key>            <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>     <string>${APP_NAME}</string>
    <key>CFBundleExecutable</key>      <string>${BINARY_NAME}</string>
    <key>CFBundleIconFile</key>        <string>${APP_NAME}</string>
    <key>CFBundleVersion</key>         <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key> <string>${VERSION}</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>CFBundleSignature</key>       <string>????</string>
    <key>NSHighResolutionCapable</key> <true/>
    <key>LSMinimumSystemVersion</key>  <string>12.0</string>
</dict>
</plist>
PLIST

    # Binary
    cp "target/release/${BINARY_NAME}" "${MACOS_DIR}/${BINARY_NAME}"

    # App icon
    make_icon

    # GTK needs a wrapper script to set up env vars before launching the binary
    mv "${MACOS_DIR}/${BINARY_NAME}" "${MACOS_DIR}/${BINARY_NAME}-bin"
    cat > "${MACOS_DIR}/${BINARY_NAME}" <<'LAUNCHER'
#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
export DYLD_LIBRARY_PATH="${DIR}/../lib:${DYLD_LIBRARY_PATH:-}"
# GTK4 needs this to find its data files when bundled
export GDK_PIXBUF_MODULEDIR="${DIR}/../lib/gdk-pixbuf-2.0/2.10.0/loaders"
export GDK_PIXBUF_MODULE_FILE="${DIR}/../lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
export XDG_DATA_DIRS="${DIR}/../share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
exec "${DIR}/skyterm-bin" "$@"
LAUNCHER
    chmod +x "${MACOS_DIR}/${BINARY_NAME}"

    info ".app bundle structure created"
}

# ── Bundle dylibs ────────────────────────────────────────────────────────────

bundle_dylibs() {
    info "Bundling dylibs with dylibbundler (this may take a moment)..."

    local brew_prefix
    brew_prefix="$(brew --prefix)"
    local lib_dir="${CONTENTS}/lib"
    mkdir -p "$lib_dir"

    # Fix up the actual binary, not the launcher script
    dylibbundler \
        --bundle-deps \
        --fix-file "${MACOS_DIR}/${BINARY_NAME}-bin" \
        --dest-dir "$lib_dir" \
        --install-path "@executable_path/../lib" \
        --search-path "${brew_prefix}/lib" \
        --overwrite-dir

    info "dylibs bundled ($(du -sh "$lib_dir" | cut -f1))"
}

# ── DMG ──────────────────────────────────────────────────────────────────────

make_dmg() {
    info "Creating DMG..."

    mkdir -p target/macos

    # Staging dir: only the .app goes in the DMG
    local staging="target/macos/dmg-staging"
    rm -rf "$staging"
    mkdir -p "$staging"
    cp -R "$APP_DIR" "$staging/"

    create-dmg \
        --volname "${APP_NAME}" \
        --volicon "${RESOURCES_DIR}/${APP_NAME}.icns" \
        --window-pos 200 120 \
        --window-size 600 400 \
        --icon-size 100 \
        --icon "${APP_NAME}.app" 175 190 \
        --hide-extension "${APP_NAME}.app" \
        --app-drop-link 425 190 \
        "$DMG_OUT" \
        "$staging"

    rm -rf "$staging"

    local size
    size=$(du -sh "$DMG_OUT" | cut -f1)
    info "DMG created: $DMG_OUT ($size)"

    echo
    echo -e "${GREEN}Done.${NC}"
    echo
    echo "  Open:       open $DMG_OUT"
    echo "  Install:    drag ${APP_NAME}.app to /Applications"
    echo
}

# ── Main ─────────────────────────────────────────────────────────────────────

check_deps
build_release
make_app_bundle
bundle_dylibs
make_dmg
