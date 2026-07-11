# Maintainer: Aner <aner@example.com>
pkgname=recomplog
pkgver=0.1.0
pkgrel=1
pkgdesc="Unified local CLI for body recomposition tracking: workouts, body metrics, sleep, nutrition, and reports"
arch=('x86_64')
url="https://github.com/argent0/recomplog"
license=('MIT')
depends=('gcc-libs')
provides=('recomplog')
makedepends=('git' 'rust' 'cargo')
source=("${pkgname}::git+ssh://git@github.com/argent0/recomplog.git")
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
  install -Dm755 "target/release/recomplog" "$pkgdir/usr/bin/recomplog"
  install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 docs/*.md -t "$pkgdir/usr/share/doc/$pkgname/"
  if [[ -f LICENSE ]]; then
    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  fi
}
