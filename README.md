# Mullvad-Gui-Slint

An independent Rust/Slint frontend for the Mullvad VPN daemon on Linux. It uses
the daemon's gRPC management socket directly and does not invoke or parse the
`mullvad` CLI.

The current implementation covers tunnel control and live state updates, relay
selection, account and device management, vouchers, core and advanced VPN
settings, and Linux split-tunnel process exclusions. Remaining Electron-app
parity is tracked explicitly in [`docs/parity.md`](docs/parity.md).

The Linux desktop integration includes the animated Mullvad tray icon, tray
tunnel controls, state notifications, close-to-tray behavior, and XDG
autostart. Start without presenting the window with `mullvad-gui-slint --background`.
When no tray host is available, closing the window exits normally so the app
cannot become inaccessible. Notification delivery is optional and never
blocks tunnel control.

This is an independent frontend, not an official Mullvad VPN application.

## Prerequisites

Install Rust and the native build dependencies for your distribution:

```text
Debian/Ubuntu: sudo apt install libfontconfig1 libxkbcommon0 build-essential
Fedora:        sudo dnf install fontconfig libxkbcommon gcc
Arch Linux:    sudo pacman -S fontconfig libxkbcommon base-devel
```

The official Mullvad VPN daemon must be installed and running for the app to
show state or control the tunnel.

For a reproducible development environment, enter the Nix shell. It includes
the pinned Rust toolchain, native libraries, and `slint-lsp`:

```sh
nix develop
```

## Run

```sh
cargo run
```

## Test

The default suite uses a fake daemon and does not change the machine's VPN
state:

```sh
cargo test
cargo fmt --check
slint-lsp format -i ui/*.slint
cargo clippy --all-targets --all-features -- -D warnings
```

Controller and daemon integration tests can run without the Slint UI dependencies:

```sh
cargo test --no-default-features
```

The ignored live integration tests perform read-only requests against the
installed daemon socket:

```sh
cargo test --no-default-features --test controller_integration -- --ignored
```

GitHub Actions runs formatting, tests, Clippy, native x86_64 and ARM64 builds,
Nix builds, and release packaging inside Docker containers. Tagged releases produce `.deb`,
`.rpm`, AUR recipe, AppImage, and portable tar artifacts for both architectures;
GitHub supplies the source archives automatically. The repository also exposes
`packages.x86_64-linux.default` and `packages.aarch64-linux.default` through
[`flake.nix`](flake.nix). `protoc` is vendored through Cargo, so CI does not
depend on a system Protobuf compiler.

Update availability is read from this project's GitHub Releases feed. Stable
builds ignore prereleases unless the beta program is enabled; alpha, beta, and
release-candidate builds continue following prereleases automatically. Failure
to reach GitHub never blocks startup or tunnel control.

A scheduled workflow installs the latest packages from Mullvad's official
stable and beta Debian repositories, starts `mullvad-daemon` directly, and runs
the read-only socket tests every day. It never invokes the Mullvad CLI.

## Desktop compatibility

The main window and GIO links work on X11 and Wayland without a particular
window manager. The tray uses StatusNotifierItem; desktops without a tray host
keep normal close-to-exit behavior. Notifications use the standard desktop
notification service and degrade silently when no notification daemon exists.
See [`docs/desktop-compatibility.md`](docs/desktop-compatibility.md).

## Support

Use the in-app **Support and report an issue** page or the
[GitHub issue tracker](https://github.com/Greenstorm5417/Mullvad-Gui-Slint/issues) for
Mullvad-Gui-Slint bugs. Use [Mullvad support](https://mullvad.net/help) for VPN
service, billing, and account issues. Never post an account number or voucher.

## Upstream and license

This project is GPL-3.0-or-later. The UI assets and vendored daemon protocol
come from the official
[Mullvad VPN application](https://github.com/mullvad/mullvadvpn-app). See
[`UPSTREAM.md`](UPSTREAM.md) for the exact reference commit and attribution.
