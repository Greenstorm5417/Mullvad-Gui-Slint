# Changelog

## 0.1.0-beta.1 - 2026-07-22

- Fixed daemon-compatibility CI: the isolated test image now stubs `systemctl`
  and ships `libdbus-1-3` so the `mullvad-vpn` package installs and the daemon
  starts cleanly during the daily compatibility check.

## 0.1.0-alpha.1 - 2026-07-20

- Initial native Slint application shell and direct Mullvad daemon gRPC client.
- Tunnel, relay, account, device, voucher, VPN setting, and split-tunnel controls.
- Animated map, tray indicator, desktop notifications, and XDG autostart.
- Close-to-tray lifecycle with working tray reopen and explicit quit actions.
- Update discovery from the Mullvad-Gui-Slint GitHub Releases feed.
- Full problem-report send states, warnings, status feedback, and beta-setting guidance.
- Debian, RPM, AUR, Nix, AppImage, and portable tar packaging for x86_64 and ARM64.
- Containerized CI and daily compatibility checks against current Mullvad packages.

This is a prerelease. Remaining behavior differences are tracked in
[`docs/parity.md`](docs/parity.md).
