# Desktop parity tracker

The Electron application is the behavioral reference. Checked items are
implemented against the daemon API; the integration suite covers the shared
controller boundary, while protobuf mapping tests cover daemon data models.
Unchecked items are not yet at feature parity.

## Connection

- [x] Read tunnel state directly from the daemon
- [x] Connect and disconnect
- [x] Reconnect and live daemon event stream
- [x] Relay and location selection
- [ ] Recent and custom locations
- [ ] Location filters and multihop selection

## Account

- [x] Login, logout, and account creation
- [x] Account expiry and voucher redemption
- [ ] Device rename
- [x] Device list and removal
- [ ] Web account authentication

## VPN settings

- [x] Auto-connect and lockdown mode
- [x] Local network sharing and IPv6
- [x] DNS content blockers
- [ ] Custom DNS
- [ ] WireGuard ports and allowed IPs
- [x] WireGuard MTU
- [x] Quantum resistance and DAITA
- [x] Multihop mode and anti-censorship transports
- [ ] Server IP overrides and settings import/export

## Linux desktop

- [x] Split tunneling process exclusion
- [x] Animated tray indicator, tunnel controls, and background behavior
- [x] Tunnel-state desktop notifications with an enable/disable control
- [x] XDG autostart and background launch
- [x] In-app Mullvad-GTK issue reporting links
- [ ] Diagnostic log collection, changelog view, and app updates
- [ ] Localization, accessibility, and keyboard navigation

## Visual parity

- [x] Upstream window geometry, color and typography tokens, spacing, and button states
- [x] Animated globe, daemon location marker, translucent connection panel, and page transitions
- [x] Upstream logo, fonts, icons, illustrations, and Linux tray animation frames
- [ ] Pixel-level review of every secondary view and transient state

## Distribution

- [x] Debian and RPM packages for x86_64 and ARM64
- [x] AUR recipe for x86_64 and aarch64
- [x] Nix flake packages for x86_64-linux and aarch64-linux
- [x] AppImage and portable tar packages for x86_64 and ARM64
- [x] GitHub releases with automatic source archives
