FROM rust:1.97-bookworm

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        desktop-file-utils \
        file \
        libfuse2 \
        libfontconfig1-dev \
        libfreetype6-dev \
        libxkbcommon-dev \
        patchelf \
        pkg-config \
        rpm \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add clippy rustfmt

WORKDIR /workspace
