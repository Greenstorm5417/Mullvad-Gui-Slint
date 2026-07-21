# Contributing

Use Rust 1.96.1 and install the GTK4 development package for your distribution.
Before opening a pull request, run:

```sh
cargo fmt --all --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Daemon protocol changes must keep `proto/` and the attribution in `UPSTREAM.md`
aligned with one upstream Mullvad commit. Tests must not connect, disconnect,
change settings, or require a real account unless they are explicitly isolated
and documented. Never include account numbers or voucher codes in fixtures.
