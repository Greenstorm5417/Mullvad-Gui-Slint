FROM rust:1.96-bookworm

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        desktop-file-utils \
        file \
        libfuse2 \
        libgtk-4-dev \
        patchelf \
        pkg-config \
        rpm \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add clippy rustfmt

WORKDIR /workspace
