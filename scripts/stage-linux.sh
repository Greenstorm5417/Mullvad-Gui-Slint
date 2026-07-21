#!/usr/bin/env bash
set -euo pipefail

root="${DESTDIR:?DESTDIR must point at a staging root}"
prefix="${PREFIX-/usr}"
binary="${BINARY_PATH:-target/release/mullvad-gui-slint}"

if [[ "${SKIP_BINARY_INSTALL:-0}" != 1 ]]; then
  install -Dm755 "$binary" "$root$prefix/bin/mullvad-gui-slint"
fi
install -Dm644 packaging/mullvad-gui-slint.desktop \
  "$root$prefix/share/applications/mullvad-gui-slint.desktop"
install -Dm644 packaging/io.github.Greenstorm5417.MullvadGuiSlint.metainfo.xml \
  "$root$prefix/share/metainfo/io.github.Greenstorm5417.MullvadGuiSlint.metainfo.xml"
install -Dm644 assets/images/logo-icon.svg \
  "$root$prefix/share/icons/hicolor/scalable/apps/mullvad-gui-slint.svg"
install -Dm644 LICENSE.md "$root$prefix/share/licenses/mullvad-gui-slint/LICENSE.md"
install -Dm644 README.md "$root$prefix/share/doc/mullvad-gui-slint/README.md"

install -d "$root$prefix/share/mullvad-gui-slint/tray"
install -m644 assets/images/menubar-icons/linux/*.png \
  "$root$prefix/share/mullvad-gui-slint/tray/"
install -d "$root$prefix/share/fonts/mullvad-gui-slint"
install -m644 assets/fonts/*.ttf "$root$prefix/share/fonts/mullvad-gui-slint/"
