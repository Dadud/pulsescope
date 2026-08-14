# syntax=docker/dockerfile:1.7

FROM node:22-bookworm-slim AS ui-builder

ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH

WORKDIR /build
RUN corepack enable

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY ui/package.json ui/package.json
RUN pnpm install --frozen-lockfile --filter pulsescope-ui...

COPY ui ui
COPY scripts scripts
RUN pnpm build

FROM rust:1-bookworm AS server-builder

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_TERM_COLOR=never

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        clang \
        cmake \
        libasound2-dev \
        libhidapi-libusb0 \
        libclang-dev \
        libssl-dev \
        libsoapysdr-dev \
        libudev-dev \
        libusb-1.0-0-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY src-tauri src-tauri
COPY release release

RUN cargo build \
    --manifest-path src-tauri/Cargo.toml \
    --release \
    --no-default-features \
    --features "headless,mock-source,soapysdr" \
    --bin pulsescope

FROM debian:bookworm-slim AS runtime

ENV DEBIAN_FRONTEND=noninteractive
ENV PULSESCOPE_DATA_DIR=/var/lib/pulsescope
ENV PULSESCOPE_BIND=0.0.0.0
ENV PULSESCOPE_PORT=8765
ENV PULSESCOPE_UI_DIR=/app/ui
ENV PULSESCOPE_AUDIO_OUTPUT=0

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libasound2 \
        libhidapi-libusb0 \
        libsoapysdr0.8 \
        libssl3 \
        libudev1 \
        libusb-1.0-0 \
        soapysdr-module-all \
        soapysdr-tools \
        wsjtx \
        tini \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 500 pulsescope \
    && useradd --uid 500 --gid pulsescope --home-dir "$PULSESCOPE_DATA_DIR" --shell /usr/sbin/nologin pulsescope \
    && mkdir -p "$PULSESCOPE_DATA_DIR/decoders" "$PULSESCOPE_DATA_DIR/recordings" /app/ui \
    && chown -R pulsescope:pulsescope "$PULSESCOPE_DATA_DIR"

COPY --from=server-builder /build/src-tauri/target/release/pulsescope /usr/local/bin/pulsescope
COPY --from=ui-builder /build/ui/build /app/ui
COPY deploy/preflight.sh /usr/local/lib/pulsescope/preflight.sh

USER pulsescope
WORKDIR /app

EXPOSE 8765
VOLUME ["/var/lib/pulsescope"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8765/health/ready >/dev/null || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/pulsescope", "--server"]
