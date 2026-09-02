# Maintainer: skyterm contributors
#
# Builds skyterm from this checkout — the same tree the .deb and .rpm are built
# from, so there is nothing to keep in sync but this file.
#
# Local build:   makepkg -f            (then: pacman -U skyterm-*.pkg.tar.zst)
# CI:            .github/workflows/release.yml, job `arch`
#
# This is a repo-local PKGBUILD, not an AUR one: it compiles what is on disk
# instead of fetching a source tarball, which is what makes it usable both for
# `makepkg -f` in a working tree and for the release pipeline.

pkgname=skyterm
# Single source of truth: the workspace version in Cargo.toml. Falls back to
# 0 so a parse failure surfaces as an obviously wrong package rather than a
# silently stale version. BASH_SOURCE (not $startdir) so this works while
# makepkg is still sourcing the file.
_repo="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pkgver="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p}' \
    "$_repo/Cargo.toml" | head -1)"
pkgver="${pkgver:-0}"
pkgrel=1
pkgdesc="GPU-rendered terminal emulator with tabs, splits and themes"
arch=('x86_64' 'aarch64')
url="https://github.com/perfecto25/skyterm"
license=('MIT')
# Runtime: GTK4 pulls in glib/pango/cairo itself, but skyterm links epoxy,
# freetype and fontconfig directly, so they are named here.
depends=('gtk4' 'libepoxy' 'freetype2' 'fontconfig')
makedepends=('cargo' 'pkgconf')
# No separate -debug package: the release profile carries no useful debug info,
# so it comes out near-empty and would only clutter the release artifacts.
options=('!debug')
# The tree is built in place (no source array), so makepkg has nothing to
# fetch or verify.
source=()
sha256sums=()

build() {
    cd "$_repo"
    # Match the release workflow: no -C target-cpu=native, which would bake in
    # the build machine's CPU features and SIGILL on older hardware.
    export RUSTUP_TOOLCHAIN=stable
    cargo build --release --locked --package skyterm-gui
}

check() {
    cd "$_repo"
    cargo test --locked --package skyterm-core
}

package() {
    cd "$_repo"
    install -Dm755 target/release/skyterm "$pkgdir/usr/bin/skyterm"
    install -Dm644 skyterm-gui/resources/skyterm.desktop \
        "$pkgdir/usr/share/applications/skyterm.desktop"
    install -Dm644 skyterm-gui/resources/skyterm.svg \
        "$pkgdir/usr/share/icons/hicolor/scalable/apps/skyterm.svg"
    install -Dm644 LICENSE-MIT "$pkgdir/usr/share/licenses/$pkgname/LICENSE-MIT"
}
