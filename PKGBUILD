# Maintainer: Raj Chauhan <rajsinghchauhan352@gmail.com>
pkgname=walllust
pkgver=0.2.0
pkgrel=1
pkgdesc="A wallpaper daemon and GUI for Wayland/Hyprland with image transitions, video wallpapers, interactive scenes and Pywal support"
arch=('x86_64')
url="https://github.com/rajchauhan28/WallLust"
license=('MIT')
# Derived from `ldd` on the release binaries. Slint is statically linked, so
# there is no slint package to depend on; qt6-base is pulled in because the
# slint crate is built with its default features, which include the Qt
# backend. webkitgtk-6.0 and gtk4-layer-shell are needed by walllust-renderer.
depends=('gcc-libs' 'glibc' 'qt6-base' 'ffmpeg' 'webkitgtk-6.0' 'gtk4'
         'gtk4-layer-shell' 'libxkbcommon' 'fontconfig' 'freetype2'
         'harfbuzz' 'libglvnd' 'libx11' 'glib2' 'icu' 'libsoup3' 'libpng')
optdepends=('mpvpaper: video wallpaper backend'
            'python-pywal: colour scheme generation'
            'libpipewire: parec, for audio-reactive scene wallpapers'
            'hyprland: cursor tracking in scene wallpapers')
makedepends=('rust' 'clang' 'pkgconf')
source=("$pkgname-$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
    cd "$srcdir/$pkgname-build"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    cargo build --release --locked
}

check() {
    cd "$srcdir/$pkgname-build"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    # Compile-checks ui/wallpaper.slint and scenes/demo.slint.
    cargo test --release --locked --bin walllust-daemon
}

package() {
    cd "$srcdir/$pkgname-build"
    # src/main.rs is a vestigial hello-world binary that cargo still builds as
    # `walllust`; it is deliberately not installed.
    install -Dm755 target/release/walllust-daemon   "$pkgdir/usr/bin/walllust-daemon"
    install -Dm755 target/release/walllust-cli      "$pkgdir/usr/bin/walllust-cli"
    install -Dm755 target/release/walllust-gui      "$pkgdir/usr/bin/walllust-gui"
    install -Dm755 target/release/walllust-renderer "$pkgdir/usr/bin/walllust-renderer"
    install -Dm644 walllust-daemon.service "$pkgdir/usr/lib/systemd/user/walllust-daemon.service"
    install -Dm644 walllust.desktop        "$pkgdir/usr/share/applications/walllust.desktop"
    install -Dm644 scenes/demo.slint       "$pkgdir/usr/share/walllust/scenes/demo.slint"
    install -Dm644 LICENSE   "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
