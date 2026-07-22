FROM rust:1.97-bookworm

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        gnupg \
        iproute2 \
        libdbus-1-3 \
        procps \
    && rm -rf /var/lib/apt/lists/*

# The container has no init system, so the mullvad-vpn package's postinst
# (which calls `systemctl enable/start` on the daemon unit) would otherwise
# fail with "systemctl: command not found". The test starts the daemon
# binary directly, so a no-op stub is enough to let the package configure.
RUN printf '#!/bin/sh\nexit 0\n' > /usr/bin/systemctl \
    && chmod 755 /usr/bin/systemctl

WORKDIR /workspace
