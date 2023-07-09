# syntax=docker/dockerfile-upstream:master-experimental
FROM lukemathwalker/cargo-chef:0.1.61-rust-1-slim-buster AS chef
WORKDIR /app

LABEL org.opencontainers.image.source=https://github.com/paradigmxyz/reth
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

# Builds a cargo-chef plan
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json

# Set the build profile to be release
ARG BUILD_PROFILE=release
ENV BUILD_PROFILE $BUILD_PROFILE

# Install system dependencies
RUN set -eux; \
    apt-get update && apt-get install -qqy --assume-yes --no-install-recommends \
    libclang-dev \
    pkg-config; \
    && rm -rf /var/lib/apt/lists/*;

# Builds dependencies
RUN cargo chef cook --profile $BUILD_PROFILE --recipe-path recipe.json

# Build application
COPY . .
RUN cargo build --profile $BUILD_PROFILE --locked --bin reth


FROM debian:bullseye-20220509-slim AS runtime
WORKDIR /app

# Copy reth over from the build stage
COPY --from=builder /app/target/release/reth /usr/local/bin

# Copy licenses
COPY LICENSE-* ./

SHELL ["/bin/bash", "-c"]

RUN exec "$SHELL"

# 8545 is Standard Port
# 8180 is OpenEthereum
# 3001 is a fallback port
EXPOSE 8545/tcp
EXPOSE 8545/udp
EXPOSE 8180
EXPOSE 3001/tcp

EXPOSE 30303/tcp
EXPOSE 30303/udp 
EXPOSE 9001
EXPOSE 8546

STOPSIGNAL SIGQUIT

ENTRYPOINT ["/usr/local/bin/reth"]
