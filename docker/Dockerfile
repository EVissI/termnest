# Reproducible build and test environment for termnest.
# Cross-compiles the Windows app (x86_64-pc-windows-msvc) from Linux with cargo-xwin.
# The app itself is never run inside this container.
FROM rust:1.99.0-slim-trixie

ARG NODE_VERSION=24.21.0
ARG CARGO_XWIN_VERSION=0.23.1
ARG LLVM_VERSION=19

# Be tolerant to short network hiccups while downloading crates.
ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=60

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        clang-${LLVM_VERSION} lld-${LLVM_VERSION} llvm-${LLVM_VERSION} \
        nsis ca-certificates curl xz-utils pkg-config git \
    && rm -rf /var/lib/apt/lists/*

# cargo-xwin and the Tauri bundler expect unversioned LLVM tool names
# (clang-cl, lld-link, llvm-lib, llvm-rc); Debian keeps them in /usr/lib/llvm-N/bin.
# Debian does not ship clang-cl: it is the same clang binary, the driver mode
# is selected by the executable name.
ENV PATH="/usr/lib/llvm-${LLVM_VERSION}/bin:${PATH}"
RUN ln -s "/usr/lib/llvm-${LLVM_VERSION}/bin/clang" /usr/local/bin/clang-cl \
    && clang-cl --version && lld-link --version && command -v llvm-rc llvm-lib llvm-ar

# Node.js LTS from the official tarball, verified against the published checksums.
RUN cd /tmp \
    && curl -fsSLO "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
    && curl -fsSLO "https://nodejs.org/dist/v${NODE_VERSION}/SHASUMS256.txt" \
    && grep " node-v${NODE_VERSION}-linux-x64.tar.xz\$" SHASUMS256.txt | sha256sum -c - \
    && tar -xJf "node-v${NODE_VERSION}-linux-x64.tar.xz" -C /usr/local --strip-components=1 --no-same-owner \
    && rm -f "node-v${NODE_VERSION}-linux-x64.tar.xz" SHASUMS256.txt \
    && node --version && npm --version

RUN rustup target add x86_64-pc-windows-msvc \
    && rustup component add clippy rustfmt \
    && cargo install --locked cargo-xwin --version "${CARGO_XWIN_VERSION}" \
    && rm -rf /usr/local/cargo/registry /usr/local/cargo/git

WORKDIR /src
