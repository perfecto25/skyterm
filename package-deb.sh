#!/usr/bin/env bash
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

# ── Prerequisites ────────────────────────────────────────────────────────────

check_deps() {
    info "Checking prerequisites..."

    command -v cargo &>/dev/null || die "cargo not found — install Rust via rustup"
    command -v dpkg  &>/dev/null || die "dpkg not found — are you on a Debian/Ubuntu system?"

    if ! cargo deb --version &>/dev/null; then
        warn "cargo-deb not found — installing..."
        cargo install cargo-deb
    fi

    # sudo apt update && sudo apt install -y \
    # build-essential \
    # pkg-config \
    # libgtk-4-dev \
    # libepoxy-dev \
    # libfreetype-dev \
    # libfontconfig1-dev \
    # libglib2.0-dev

    info "Prerequisites OK"
}

# ── Build ────────────────────────────────────────────────────────────────────

build_release() {
    info "Building release binary..."

    # rustc >= 1.90 defaults to the bundled rust-lld linker, which does NOT
    # search the system/multiarch lib dirs (e.g. /usr/lib/x86_64-linux-gnu on
    # Debian/Ubuntu) the way GNU ld does. That makes -lgtk-4/-lepoxy/etc.
    # "unable to find library" on Debian-family build hosts. Force the system
    # GNU linker (ld.bfd) for the deb build so the GTK4 libs resolve.
    RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-fuse-ld=bfd" \
        cargo build \
            --release \
            --package skyterm-gui \
            --locked

    local bin="target/release/skyterm"
    [[ -f "$bin" ]] || die "Binary not found at $bin after build"

    local size
    size=$(du -sh "$bin" | cut -f1)
    info "Binary built: $bin ($size)"
}

# ── Package ──────────────────────────────────────────────────────────────────

build_deb() {
    info "Generating .deb package..."

    # --no-build: binary is already compiled above; skip redundant rebuild.
    cargo deb --package skyterm-gui --no-build

    local deb
    deb=$(ls target/debian/skyterm_*.deb 2>/dev/null | head -1)
    [[ -n "$deb" ]] || die ".deb not found in target/debian/ after packaging"

    local size
    size=$(du -sh "$deb" | cut -f1)
    info ".deb created: $deb ($size)"

    info ".deb contents:"
    dpkg-deb --contents "$deb" | awk '{print $NF}' | sed 's/^/   /'

    echo
    echo -e "${GREEN}Done.${NC}"
    echo
    echo "  Install:    sudo dpkg -i $deb"
    echo "  Verify:     dpkg-deb --info $deb"
    echo "  Remove:     sudo apt remove skyterm"
    echo
}

# ── Main ─────────────────────────────────────────────────────────────────────

check_deps
build_release
build_deb
