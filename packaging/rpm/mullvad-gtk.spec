Name:           mullvad-gtk
Version:        %{package_version}
Release:        1%{?dist}
Summary:        Native GTK4 frontend for the Mullvad VPN daemon
License:        GPL-3.0-or-later
URL:            https://github.com/Greenstorm5417/Mullvad-GTK
Requires:       gtk4

%description
A native Linux frontend that controls the installed Mullvad VPN daemon through
its management socket.

%prep

%build

%install
cd %{source_root}
DESTDIR=%{buildroot} PREFIX=/usr ./scripts/stage-linux.sh

%files
%license %{_datadir}/licenses/mullvad-gtk/LICENSE.md
%doc %{_datadir}/doc/mullvad-gtk/README.md
%{_bindir}/mullvad-gtk
%{_datadir}/applications/mullvad-gtk.desktop
%{_datadir}/metainfo/io.github.Greenstorm5417.MullvadGTK.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/mullvad-gtk.svg
%{_datadir}/mullvad-gtk/tray/*.png
%{_datadir}/fonts/mullvad-gtk/*.ttf

%changelog
* Mon Jul 20 2026 Mullvad-GTK contributors - %{package_version}-1
- Initial package
