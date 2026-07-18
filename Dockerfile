# Multi-stage Dockerfile for PulseScope server.
#
# The first stages build the SvelteKit frontend and Rust backend.
# The runtime stage produces a slim Debian image containing the binary + static UI +
# SoapySDR runtime + common hardware libs. Multi-arch-safe.

# ─────────────────────────────────────────────────────────────
# Frontend build stage
# ─────────────────────────────────────────────────────────────
FROM node:20-bookworm-slim AS frontend

RUN corepack enable
WORKDIR /build
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY ui/package.json ui/package.json
RUN pnpm install --frozen-lockfile
COPY ui ui
RUN pnpm build

# ────────────────────────────────────────────────────────────
# Rust build stage
# ────────────────────────────────────────────────────────────
FROM rust:1-bookworm AS builder

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_TERM_COLOR=never

# Native development headers required by Tauri/WebKitGTK, CPAL/ALSA,
# GLib, SoapySDR, and crates that discover libraries through pkg-config.
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        cmake \
        build-essential \
        pkg-config \
        libglib2.0-dev \
        libgtk-3-dev \
        libwebkit2gtk-4.1-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libasound2-dev \
        libsoapysdr-dev \
        patchelf \
        libusb-1.0-0-dev \
        libhidapi-libusb0 \
        libudev-dev \
        libssl-dev \
        git && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Add source
COPY . .
COPY --from=frontend /build/ui/build /build/ui/build

# Build the server binary (--server mode is the default flag, but turn it on):
RUN cargo build --release --manifest-path src-tauri/Cargo.toml --features soapysdr --bin pulsescope

# ─────────────────────────────────────────────────────────────
# Runtime stage
# ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# Runtime libs needed for SoapySDR modules and USB SDR hardware
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libusb-1.0-0 \
        libhidapi-libusb0 \
        libudev1 \
        libssl3 \
        libsoapysdr0.8 \
        curl \
        tini && \
    rm -rf /var/lib/apt/lists/*

# Where data, recordings, decoders, and config live in the container
ENV PULSESCOPE_DATA_DIR=/var/lib/pulsescope
ENV PULSESCOPE_BIND=0.0.0.0
ENV PULSESCOPE_PORT=8765
ENV PULSESCOPE_AUTH_FILE=$PULSESCOPE_DATA_DIR/auth_token

RUN groupadd --gid 500 pulsescope && \
    useradd --uid 500 --gid pulsescope --home $PULSESCOPE_DATA_DIR --shell /usr/sbin/nologin pulsescope && \
    mkdir -p $PULSESCOPE_DATA_DIR/decoders $PULSESCOPE_DATA_DIR/recordings && \
    chown -R pulsescope:pulsescope $PULSESCOPE_DATA_DIR

WORKDIR /app

# Pull in the backend and the frontend produced by the two build stages.
COPY --from=builder /build/src-tauri/target/release/pulsescope /usr/local/bin/pulsescope
COPY --from=frontend /build/ui/build /app/ui/build

# Healthcheck: every 30s, expect 200 from /health
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8765/health || exit 1

EXPOSE 8765

# Drop privileges, drop nets, hand off to our process with tini for sane signal handling
USER pulsescope
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/pulsescope", "--server"]
