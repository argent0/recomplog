# Maintainer: Aner <aner@example.com>
pkgname=repslog
pkgver=0.1.0
pkgrel=1
pkgdesc="A Linux-first command-line workout tracker"
arch=('x86_64')
url="https://github.com/argent0/repslog"
license=('MIT')
depends=('gcc-libs')
provides=('repslog')
makedepends=('git' 'rust' 'cargo')
source=("${pkgname}::git+ssh://git@github.com/argent0/repslog.git")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  local _ver=$(grep '^version =' Cargo.toml | head -n 1 | cut -d '"' -f 2)
  echo "${_ver}.r$(git rev-list --count HEAD).$(git rev-parse --short HEAD)"
}

build() {
  cd "$srcdir/$pkgname"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname"
  install -Dm755 "target/release/repslog" "$pkgdir/usr/bin/repslog"
  install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 docs/*.md -t "$pkgdir/usr/share/doc/$pkgname/"
  install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
