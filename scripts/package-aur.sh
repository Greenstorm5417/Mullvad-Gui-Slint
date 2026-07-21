#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: package-aur.sh VERSION}"
pkgver="${version/-/_}"
directory="dist/mullvad-gui-slint-aur"

rm -rf "$directory"
mkdir -p "$directory"
cp packaging/PKGBUILD packaging/.SRCINFO "$directory/"
sed -i "s/^pkgver=.*/pkgver=$pkgver/" "$directory/PKGBUILD"
sed -i "s/^\tpkgver = .*/\tpkgver = $pkgver/" "$directory/.SRCINFO"
sed -i "s|^\tsource = .*|\tsource = mullvad-gui-slint-$pkgver.tar.gz::https://github.com/Greenstorm5417/Mullvad-Gui-Slint/archive/refs/tags/v$version.tar.gz|" "$directory/.SRCINFO"
tar -C dist -czf "dist/mullvad-gui-slint-aur-$version.tar.gz" mullvad-gui-slint-aur
