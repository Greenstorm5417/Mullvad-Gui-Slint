#!/usr/bin/env bash
set -euo pipefail

root="${DESTDIR:?DESTDIR must point at a staging root}"
prefix="${PREFIX-/usr}"
binary="${BINARY_PATH:-target/release/mullvad-gtk}"

install -Dm755 "$binary" "$root$prefix/bin/mullvad-gtk"
install -Dm644 packaging/mullvad-gtk.desktop \
  "$root$prefix/share/applications/mullvad-gtk.desktop"
install -Dm644 packaging/io.github.Greenstorm5417.MullvadGTK.metainfo.xml \
  "$root$prefix/share/metainfo/io.github.Greenstorm5417.MullvadGTK.metainfo.xml"
install -Dm644 assets/images/logo-icon.svg \
  "$root$prefix/share/icons/hicolor/scalable/apps/mullvad-gtk.svg"
install -Dm644 LICENSE.md "$root$prefix/share/licenses/mullvad-gtk/LICENSE.md"
install -Dm644 README.md "$root$prefix/share/doc/mullvad-gtk/README.md"

install -d "$root$prefix/share/mullvad-gtk/tray"
install -m644 assets/images/menubar-icons/linux/*.png \
  "$root$prefix/share/mullvad-gtk/tray/"
install -d "$root$prefix/share/fonts/mullvad-gtk"
install -m644 assets/fonts/*.ttf "$root$prefix/share/fonts/mullvad-gtk/"
