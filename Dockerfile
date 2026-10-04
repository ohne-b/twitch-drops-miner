ARG BUILDPLATFORM=linux/amd64
FROM --platform=$BUILDPLATFORM node:24-bookworm-slim AS dashboard
WORKDIR /build/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
COPY lang/ /build/lang/
RUN npm run build

FROM --platform=$BUILDPLATFORM rust:1.97.1-bookworm AS build
ARG TARGETARCH
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
      cmake gcc-aarch64-linux-gnu g++-aarch64-linux-gnu gcc-x86-64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src/ ./src/
COPY lang/ ./lang/
COPY --from=dashboard /build/web/ ./web/
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
RUN case "${TARGETARCH:-amd64}" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu ;; \
      *) echo "Unsupported architecture" >&2; exit 1 ;; \
    esac \
    && rustup target add "$target" \
    && cargo build --release --locked --bin twitch-drops-miner --target "$target" \
    && cp "target/$target/release/twitch-drops-miner" /twitch-drops-miner

FROM debian:bookworm-slim
ARG BUILD_DATE
ARG VCS_REF
ARG VERSION
LABEL org.opencontainers.image.created="${BUILD_DATE}" \
      org.opencontainers.image.authors="rangermix, ohne-b" \
      org.opencontainers.image.source="https://github.com/ohne-b/twitch-drops-miner" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.licenses="PolyForm-Noncommercial-1.0.0" \
      org.opencontainers.image.title="Twitch Drops Miner"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /app/data /app/logs \
    && chown 1000:1000 /app/data /app/logs
COPY --from=build /twitch-drops-miner /usr/local/bin/twitch-drops-miner
COPY LICENSE.md NOTICE.md /usr/share/licenses/twitch-drops-miner/
COPY frontend/public/assets/licenses/ /usr/share/licenses/twitch-drops-miner/dashboard/
WORKDIR /app
ENV HOST=0.0.0.0 PORT=8080 DATA_DIR=/app/data LOG_DIR=/app/logs
USER 1000:1000
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/twitch-drops-miner", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/twitch-drops-miner"]
