FROM rust:1.96-bookworm

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        gnupg \
        iproute2 \
        procps \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
