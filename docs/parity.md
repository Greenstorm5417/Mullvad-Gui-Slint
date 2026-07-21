# Mullvad-Gui-Slint parity tracker

The Electron application is the behavioral reference. Checked items are
implemented against the daemon API; the integration suite covers the shared
controller boundary, while protobuf mapping tests cover daemon data models.
Unchecked items are not yet at feature parity.

## Connection

- [x] Read tunnel state directly from the daemon
- [x] Connect and disconnect
- [x] Reconnect and live daemon event stream
- [x] Relay and location selection
- [x] Recent and custom locations, including custom-list membership editing
- [x] Location filters and multihop entry/exit selection

## Account

- [x] Login, logout, and account creation
- [x] Account expiry and voucher redemption
- [x] Device list and removal
- [x] Web account authentication

## VPN settings

- [x] Auto-connect and lockdown mode
- [x] Local network sharing and IPv6
- [x] DNS content blockers
- [x] Custom DNS
- [x] WireGuard and anti-censorship ports
- [x] WireGuard MTU
- [x] Quantum resistance and DAITA
- [x] Multihop mode and anti-censorship transports
- [x] Server IP overrides and settings import

## Linux desktop

- [x] Split tunneling process exclusion
- [x] Animated tray indicator, tunnel controls, and background behavior
- [x] Tunnel-state desktop notifications with an enable/disable control
- [x] XDG autostart and background launch
- [x] In-app Mullvad-Gui-Slint issue reporting links
- [x] In-app changelog view
- [x] Diagnostic log collection and submission through the official problem-report helper
- [x] App update discovery through Mullvad-Gui-Slint GitHub Releases
- [ ] In-app update installation
- [ ] Localization, accessibility, and keyboard navigation

## Visual parity

- [x] Upstream window geometry, color and typography tokens, spacing, and button states
- [x] Animated globe and daemon location marker
- [x] Upstream logo, fonts, icons, illustrations, and Linux tray animation frames
- [x] Page transition animations (LocationPage/SettingsView/AccountSurface slide in/out from the right over `mullvad.slint`'s main view; direction isn't yet push/pop/dismiss-aware like upstream's `TransitionType`, just a single consistent slide)
- [x] Source/layout audit of every Slint view and transient state against upstream patterns
- [ ] Pixel-capture comparison across every desktop compositor and scale factor

## Distribution

- [x] Debian and RPM packages for x86_64 and ARM64
- [x] AUR recipe for x86_64 and aarch64
- [x] Nix flake packages for x86_64-linux and aarch64-linux
- [x] AppImage and portable tar packages for x86_64 and ARM64
- [x] GitHub releases with automatic source archives
- [x] Package smoke validation and SHA-256 release checksums
