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

appdir="$PWD/target/Mullvad-GTK.AppDir"
tools="$PWD/target/appimage-tools"
rm -rf "$appdir" "$tools"
mkdir -p "$appdir" "$tools" dist
DESTDIR="$appdir" PREFIX=/usr ./scripts/stage-linux.sh
ln -s usr/bin/mullvad-gtk "$appdir/AppRun"
cp packaging/mullvad-gtk.desktop "$appdir/mullvad-gtk.desktop"
cp assets/images/logo-icon.svg "$appdir/mullvad-gtk.svg"

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
export OUTPUT="dist/Mullvad-GTK-${version}-${architecture}.AppImage"
"$linuxdeploy" \
  --appdir "$appdir" \
  --executable "$appdir/usr/bin/mullvad-gtk" \
  --desktop-file "$appdir/mullvad-gtk.desktop" \
  --icon-file "$appdir/mullvad-gtk.svg" \
  --output appimage
