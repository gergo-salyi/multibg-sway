# Maintainer: Gergő Sályi <salyigergo94@gmail.com>
pkgname=multibg-sway
pkgver=0.1.9
pkgrel=1
pkgdesc='Set a different wallpaper for the background of each Sway workspace'
arch=('x86_64' 'i686' 'pentium4' 'armv7h' 'aarch64')
url="https://github.com/gergo-salyi/multibg-sway"
license=('MIT' 'Apache')
depends=('gcc-libs' 'glibc')
makedepends=('cargo')
optdepends=('sway: window manager to set the wallpapers with')
source=("$pkgname-$pkgver.tar.gz::https://static.crates.io/crates/$pkgname/$pkgname-$pkgver.crate")
sha256sums=('17d904c958303e69978e499d8f907794cb69086f77cc11953b19d03280c5551a')

prepare() {
    cd "$pkgname-$pkgver"
    cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
    cd "$pkgname-$pkgver"
    cargo build --frozen --release
}

package() {
    cd "$pkgname-$pkgver"
    install -Dm0755 -t "$pkgdir/usr/bin/" "target/release/$pkgname"
    install -Dm644 "README.md" "$pkgdir/usr/share/doc/${pkgname}/README.md"
    install -Dm644 "LICENSE-MIT" "$pkgdir/usr/share/licenses/${pkgname}/LICENSE-MIT"
}
