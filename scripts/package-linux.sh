#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: package-linux.sh VERSION ARCH}"
architecture="${2:?usage: package-linux.sh VERSION ARCH}"
deb_version="${version/-/~}"
rpm_version="${version//-/.}"
case "$architecture" in
  x86_64)
    deb_arch=amd64
    ;;
  aarch64)
    deb_arch=arm64
    ;;
  *)
    echo "unsupported architecture: $architecture" >&2
    exit 1
    ;;
esac

rm -rf target/package-root target/rpmbuild
mkdir -p dist target/package-root/DEBIAN
DESTDIR="$PWD/target/package-root" PREFIX=/usr ./scripts/stage-linux.sh

cat > target/package-root/DEBIAN/control <<EOF
Package: mullvad-gtk
Version: $deb_version
Section: net
Priority: optional
Architecture: $deb_arch
Depends: libgtk-4-1
Maintainer: Mullvad-GTK contributors
Description: Native GTK4 frontend for the Mullvad VPN daemon
 Uses the daemon management socket directly and never invokes the Mullvad CLI.
EOF
dpkg-deb --root-owner-group --build target/package-root \
  "dist/mullvad-gtk_${version}_${deb_arch}.deb"

mkdir -p target/rpmbuild/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
rpmbuild -bb packaging/rpm/mullvad-gtk.spec \
  --define "_topdir $PWD/target/rpmbuild" \
  --define "package_version $rpm_version" \
  --define "source_root $PWD"
find target/rpmbuild/RPMS -type f -name '*.rpm' -exec cp {} dist/ \;

tar -C target/package-root/usr -czf \
  "dist/mullvad-gtk_${version}_${architecture}.tar.gz" .
