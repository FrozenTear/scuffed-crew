# ── Stage 1: Builder ───────────────────────────────────────
# Prefer prebuilt images from GHCR (see .github/workflows/publish-image.yml)
# rather than compiling this on a small VPS.
FROM rust:1.92-bookworm AS builder

# Toolchain layers stay cached across app code changes (Buildx / GHA cache).
RUN rustup target add wasm32-unknown-unknown
RUN cargo install --locked dioxus-cli@0.7.9

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY Dioxus.toml ./
COPY crates/ crates/

# Build the Dioxus app bundle (Dioxus.toml at monorepo root supplies title/meta)
RUN cd crates/app && dx build --release

# Stage the web bundle where the server's SPA fallback expects it
RUN cp -r target/dx/scuffed-app/release/web/public dist

# Build the unified server binary (REST + strategy WebSocket + chat)
RUN cargo build --release -p scuffed-server

# ── Stage 2: Runtime ───────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home scuffed
USER scuffed
WORKDIR /app

COPY --from=builder /build/target/release/scuffed-server ./scuffed-server
COPY --from=builder /build/dist/ ./dist/

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/api/health || exit 1

ENTRYPOINT ["./scuffed-server"]
