# Maintainer: Raj Chauhan <raj@example.com>
pkgname=walllust
pkgver=0.1.0
pkgrel=1
pkgdesc="A wallpaper daemon and GUI for Wayland/Hyprland with Pywal support"
arch=('x86_64')
url="https://github.com/rajchauhan28/WallLust"
license=('MIT')
depends=('slint' 'mpvpaper' 'ffmpeg' 'python-pywal')
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
    cd "$pkgname-$pkgver"
    cargo build --release --locked
}

package() {
    cd "$pkgname-$pkgver"
    install -Dm755 target/release/walllust-daemon "$pkgdir/usr/bin/walllust-daemon"
    install -Dm755 target/release/walllust-cli "$pkgdir/usr/bin/walllust-cli"
    install -Dm755 target/release/walllust-gui "$pkgdir/usr/bin/walllust-gui"
    install -Dm644 walllust-daemon.service "$pkgdir/usr/lib/systemd/user/walllust-daemon.service"
    install -Dm644 walllust.desktop "$pkgdir/usr/share/applications/walllust.desktop"
}
