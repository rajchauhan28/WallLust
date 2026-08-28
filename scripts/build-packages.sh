#!/usr/bin/env bash
#
# Build WallLust packages for Arch, Debian and Fedora.
#
# Every package is built inside that distribution's own container, so each
# binary links against the glibc it will actually run against. Building all
# three on the host would produce .deb and .rpm files carrying Arch's much
# newer glibc, which fail at startup on the target distro with
# "version `GLIBC_2.xx' not found".
#
# The Debian image is trixie, not bookworm: walllust-renderer needs
# gtk4-layer-shell, which bookworm does not package at all, and the gtk4
# crate is built with feature v4_12 while bookworm ships GTK 4.8.
#
# Usage: scripts/build-packages.sh [arch|deb|rpm|all]   (default: all)

set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

PKGNAME=walllust
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)"/\1/')"
DIST="$REPO_ROOT/dist"
TARGET="${1:-all}"

echo "==> $PKGNAME $VERSION  (target: $TARGET)"

if [ -n "$(git status --porcelain)" ]; then
    echo "!!  Working tree is dirty. Packages are built from HEAD, so"
    echo "!!  uncommitted changes will NOT be included." >&2
fi

mkdir -p "$DIST" build
# Build from a clean HEAD archive. Never hand the working tree to Docker:
# target/ alone is several GB and would be copied into every container.
SRC_TAR="$REPO_ROOT/build/${PKGNAME}-${VERSION}.tar.gz"
git archive --format=tar.gz --prefix="${PKGNAME}-build/" -o "$SRC_TAR" HEAD
echo "==> source archive: $(du -h "$SRC_TAR" | cut -f1)"

RUSTUP='curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null 2>&1; . "$HOME/.cargo/env"'

run_in() {   # run_in <image> <script>
    docker run --rm \
        -v "$SRC_TAR:/src.tar.gz:ro" \
        -v "$DIST:/out" \
        -e VERSION="$VERSION" \
        "$1" bash -euo pipefail -c "$2"
}

build_deb() {
    echo "==> Debian package (debian:trixie)"
    run_in debian:trixie "
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        apt-get install -y -qq build-essential curl ca-certificates pkg-config \
            clang libclang-dev libwayland-dev libxkbcommon-dev libegl1-mesa-dev \
            libfontconfig1-dev libgtk-4-dev libgtk4-layer-shell-dev \
            libwebkitgtk-6.0-dev qt6-base-dev libavutil-dev libavcodec-dev \
            libavformat-dev libswscale-dev libavdevice-dev libavfilter-dev \
            libpostproc-dev libswresample-dev >/dev/null
        $RUSTUP
        cargo install cargo-deb --locked >/dev/null 2>&1
        tar xzf /src.tar.gz -C /tmp
        cd /tmp/${PKGNAME}-build
        cargo deb --output /out/
        chmod 644 /out/*.deb
    "
}

build_rpm() {
    echo "==> Fedora package (fedora:latest)"
    run_in fedora:latest "
        dnf install -y -q gcc gcc-c++ curl clang clang-devel pkgconf-pkg-config \
            rpm-build wayland-devel libxkbcommon-devel mesa-libEGL-devel \
            fontconfig-devel gtk4-devel gtk4-layer-shell-devel \
            webkitgtk6.0-devel qt6-qtbase-devel ffmpeg-free-devel >/dev/null
        $RUSTUP
        cargo install cargo-generate-rpm --locked >/dev/null 2>&1
        tar xzf /src.tar.gz -C /tmp
        cd /tmp/${PKGNAME}-build
        cargo build --release
        strip -s target/release/walllust-daemon target/release/walllust-cli \
                 target/release/walllust-gui target/release/walllust-renderer || true
        cargo generate-rpm --output /out/
        chmod 644 /out/*.rpm
    "
}

build_arch() {
    echo "==> Arch package (archlinux:base-devel)"
    # makepkg verifies that everything in depends= is installed before it will
    # build, so installing them here doubles as a check that every dependency
    # name in the PKGBUILD is a real Arch package. makepkg also refuses to run
    # as root, hence the dedicated unprivileged user.
    run_in archlinux:base-devel "
        pacman -Syu --noconfirm --needed base-devel git rust clang pkgconf \
            gcc-libs glibc qt6-base ffmpeg webkitgtk-6.0 gtk4 gtk4-layer-shell \
            libxkbcommon fontconfig freetype2 harfbuzz libglvnd libx11 glib2 \
            icu libsoup3 libpng >/dev/null
        useradd -m builder
        cp /src.tar.gz /home/builder/${PKGNAME}-\$VERSION.tar.gz
        tar xzf /src.tar.gz -C /tmp
        cp /tmp/${PKGNAME}-build/PKGBUILD /home/builder/
        chown -R builder:builder /home/builder
        su builder -c 'cd /home/builder && makepkg -f --noconfirm --skipinteg'
        cp /home/builder/*.pkg.tar.zst /out/
        chmod 644 /out/*.pkg.tar.zst
    "
}

case "$TARGET" in
    deb)  build_deb ;;
    rpm)  build_rpm ;;
    arch) build_arch ;;
    all)  build_arch; build_deb; build_rpm ;;
    *)    echo "Unknown target: $TARGET (use arch|deb|rpm|all)" >&2; exit 1 ;;
esac

echo
echo "==> Packages in dist/"
ls -lh "$DIST"
