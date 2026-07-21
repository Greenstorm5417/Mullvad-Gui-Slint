#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: package-appimage.sh VERSION ARCH}"
architecture="${2:?usage: package-appimage.sh VERSION ARCH}"
case "$architecture" in
  x86_64) appimage_arch=x86_64 ;;
  aarch64) appimage_arch=aarch64 ;;
  *)
    echo "unsupported architecture: $architecture" >&2
    exit 1
    ;;
esac

appdir="$PWD/target/Mullvad-Gui-Slint.AppDir"
tools="$PWD/target/appimage-tools"
rm -rf "$appdir" "$tools"
mkdir -p "$appdir" "$tools" dist
DESTDIR="$appdir" PREFIX=/usr ./scripts/stage-linux.sh
ln -s usr/bin/mullvad-gui-slint "$appdir/AppRun"
cp packaging/mullvad-gui-slint.desktop "$appdir/mullvad-gui-slint.desktop"
cp assets/images/logo-icon.svg "$appdir/mullvad-gui-slint.svg"

linuxdeploy="$tools/linuxdeploy-$appimage_arch.AppImage"
plugin="$tools/linuxdeploy-plugin-appimage-$appimage_arch.AppImage"
curl --fail --location --retry 3 \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$appimage_arch.AppImage" \
  --output "$linuxdeploy"
curl --fail --location --retry 3 \
  "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-$appimage_arch.AppImage" \
  --output "$plugin"
chmod +x "$linuxdeploy" "$plugin"

export APPIMAGE_EXTRACT_AND_RUN=1
export OUTPUT="dist/Mullvad-Gui-Slint-${version}-${architecture}.AppImage"
"$linuxdeploy" \
  --appdir "$appdir" \
  --executable "$appdir/usr/bin/mullvad-gui-slint" \
  --desktop-file "$appdir/mullvad-gui-slint.desktop" \
  --icon-file "$appdir/mullvad-gui-slint.svg" \
  --output appimage
