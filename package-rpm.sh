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

    command -v cargo        &>/dev/null || die "cargo not found — install Rust via rustup"
    command -v rpm          &>/dev/null || die "rpm not found — install rpm-build: sudo dnf install rpm-build"
    command -v rpmbuild     &>/dev/null || die "rpmbuild not found — sudo dnf install rpm-build"

    if ! cargo generate-rpm --version &>/dev/null; then
        warn "cargo-generate-rpm not found — installing..."
        cargo install cargo-generate-rpm
    fi

    info "Prerequisites OK"
}

# ── Build ────────────────────────────────────────────────────────────────────

build_release() {
    info "Building release binary..."

    # RUSTFLAGS: target the host CPU for maximum optimization.
    # Remove -C target-cpu=native if building for distribution on other machines.
    # RUSTFLAGS="-C target-cpu=native" \
    #     cargo build \
    #         --release \
    #         --package skyterm-gui \
    #         --locked

    RUSTFLAGS="-C target-cpu=native" \
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

build_rpm() {
    info "Generating RPM..."

    cargo generate-rpm --package skyterm-gui

    local rpm
    rpm=$(ls target/generate-rpm/skyterm-*.rpm 2>/dev/null | head -1)
    [[ -n "$rpm" ]] || die "RPM not found in target/generate-rpm/ after packaging"

    local size
    size=$(du -sh "$rpm" | cut -f1)
    info "RPM created: $rpm ($size)"

    # Print package contents so it's easy to verify assets landed correctly
    info "RPM contents:"
    rpm -qlp "$rpm" | sed 's/^/   /'

    echo
    echo -e "${GREEN}Done.${NC}"
    echo
    echo "  Install:    sudo rpm -ivh $rpm"
    echo "  Upgrade:    sudo rpm -Uvh $rpm"
    echo "  Verify:     rpm -qip $rpm"
    echo
}

# ── Main ─────────────────────────────────────────────────────────────────────

check_deps
build_release
build_rpm
