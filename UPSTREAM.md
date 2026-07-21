# Upstream attribution

This project is an independent GTK4 frontend based on the GPL-3.0-or-later
Mullvad VPN application.

- Upstream: <https://github.com/mullvad/mullvadvpn-app>
- Reference commit: `2374f6960f57ed8d4bc4def1b8dadf3f7cd6a494`
- Upstream copyright: Mullvad VPN AB and contributors
- License: GPL-3.0-or-later, included in `LICENSE.md`

The files under `assets/` were copied from
`desktop/packages/mullvad-vpn/assets/` at the reference commit. The files under
`proto/` were copied from `mullvad-management-interface/proto/` at the same
commit. `assets/geo/countries.geo.json` comes from `ios/Assets/` and drives the
native animated globe. Keep the asset and protocol reference together when
updating upstream.
