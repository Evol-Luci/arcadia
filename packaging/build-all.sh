#!/usr/bin/env bash
#
# Build every Linux package for Arcadia.
#
#   ./packaging/build-all.sh [all|tauri|pacman|flatpak]
#
#   tauri    -> appimage + deb + rpm   (Tauri bundler; needs rpmbuild for rpm)
#   pacman   -> .pkg.tar.zst           (makepkg, from the current git HEAD)
#   flatpak  -> .flatpak               (flatpak-builder, GNOME runtime)
#   all      -> the above, in order    (default)
#
# The pacman build packages the current committed tree (git archive of HEAD),
# so it works before a release is published. The committed PKGBUILD points at
# the GitHub release tarball for AUR users.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PKGDIR="$ROOT/packaging"
VERSION="$(grep -m1 '^pkgver=' "$PKGDIR/PKGBUILD" | cut -d= -f2)"

build_tauri() {
  echo "==> Tauri bundles (appimage, deb, rpm)"
  ( cd "$ROOT/desktop" && npm ci && npm run release )
  echo "    -> $ROOT/target/release/bundle/{appimage,deb,rpm}/"
}

build_pacman() {
  echo "==> pacman package (makepkg, from git HEAD)"
  local work; work="$(mktemp -d)"
  git -C "$ROOT" archive --prefix="arcadia-$VERSION/" HEAD \
    -o "$work/arcadia-$VERSION.tar.gz"
  cp "$PKGDIR/PKGBUILD" "$work/PKGBUILD"
  # build from the local tarball instead of the (maybe-unpublished) GitHub one
  sed -i "s|^source=.*|source=(\"arcadia-$VERSION.tar.gz\")|" "$work/PKGBUILD"
  ( cd "$work" && makepkg -f )
  cp "$work"/arcadia-*.pkg.tar.zst "$PKGDIR/" && rm -rf "$work"
  echo "    -> $PKGDIR/arcadia-$VERSION-*.pkg.tar.zst"
}

build_flatpak() {
  echo "==> Flatpak"
  [ -x "$ROOT/target/release/arcadia" ] || {
    echo "    error: target/release/arcadia missing; run '$0 tauri' first" >&2
    exit 1
  }
  ( cd "$PKGDIR/flatpak" && flatpak-builder --force-clean --user \
      --install-deps-from=flathub --repo="$PKGDIR/repo" \
      build org.arcadia.dreamshell.yml )
  flatpak build-bundle "$PKGDIR/repo" \
    "$PKGDIR/org.arcadia.dreamshell.flatpak" org.arcadia.dreamshell
  echo "    -> $PKGDIR/org.arcadia.dreamshell.flatpak"
}

case "${1:-all}" in
  tauri)   build_tauri ;;
  pacman)  build_pacman ;;
  flatpak) build_flatpak ;;
  all)     build_tauri; build_pacman; build_flatpak ;;
  *) echo "usage: $0 [all|tauri|pacman|flatpak]" >&2; exit 1 ;;
esac

echo "Done."
