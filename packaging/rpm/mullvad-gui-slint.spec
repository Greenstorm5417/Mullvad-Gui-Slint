Name:           mullvad-gui-slint
Version:        %{package_version}
Release:        1%{?dist}
Summary:        Native Slint frontend for the Mullvad VPN daemon
License:        GPL-3.0-or-later
URL:            https://github.com/Greenstorm5417/Mullvad-Gui-Slint
Requires:       fontconfig
Requires:       libxkbcommon
Suggests:       mullvad-vpn

%description
A native Linux frontend that controls the installed Mullvad VPN daemon through
its management socket.

%prep

%build

%install
cd %{source_root}
DESTDIR=%{buildroot} PREFIX=/usr ./scripts/stage-linux.sh

%files
%license %{_datadir}/licenses/mullvad-gui-slint/LICENSE.md
%doc %{_datadir}/doc/mullvad-gui-slint/README.md
%{_bindir}/mullvad-gui-slint
%{_datadir}/applications/mullvad-gui-slint.desktop
%{_datadir}/metainfo/io.github.Greenstorm5417.MullvadGuiSlint.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/mullvad-gui-slint.svg
%{_datadir}/mullvad-gui-slint/tray/*.png
%{_datadir}/fonts/mullvad-gui-slint/*.ttf

%changelog
* Mon Jul 20 2026 Mullvad-Gui-Slint contributors - %{package_version}-1
- Initial package
