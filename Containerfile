# ── Stage 1: Builder ───────────────────────────────────────
FROM rust:1.92-bookworm AS builder

# Install wasm target and trunk
RUN rustup target add wasm32-unknown-unknown \
    && cargo install --locked trunk@0.21.14

WORKDIR /build

# ── Dependency cache layer ─────────────────────────────────
# Copy manifests + lockfile first so deps are cached across source changes.
COPY Cargo.toml Cargo.lock ./
COPY crates/auth/Cargo.toml crates/auth/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/ui/Cargo.toml crates/ui/Cargo.toml
COPY crates/site/Cargo.toml crates/site/Cargo.toml
COPY crates/admin/Cargo.toml crates/admin/Cargo.toml
COPY crates/site-server/Cargo.toml crates/site-server/Cargo.toml

# Dummy source files so cargo can resolve the workspace and cache deps.
RUN mkdir -p crates/auth/src crates/db/src crates/ui/src \
             crates/site/src crates/admin/src crates/site-server/src \
    && echo "" > crates/auth/src/lib.rs \
    && echo "" > crates/db/src/lib.rs \
    && echo "" > crates/ui/src/lib.rs \
    && echo "fn main(){}" > crates/site/src/main.rs \
    && echo "fn main(){}" > crates/admin/src/main.rs \
    && echo "" > crates/site-server/src/lib.rs \
    && echo "fn main(){}" > crates/site-server/src/main.rs \
    && cargo build --release -p scuffed-site-server 2>/dev/null || true

# ── Copy real source ───────────────────────────────────────
COPY crates/ crates/

# Touch source files to invalidate the dummy builds.
RUN find crates -name '*.rs' -exec touch {} +

# Build site WASM
RUN cd crates/site && trunk build --release

# Build admin WASM
RUN cd crates/admin && trunk build --release

# Build server binary
RUN cargo build --release -p scuffed-site-server

# ── Stage 2: Runtime ───────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home scuffed
USER scuffed
WORKDIR /app

COPY --from=builder /build/target/release/scuffed-site-server ./scuffed-site-server
COPY --from=builder /build/dist/ ./dist/

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/api/health || exit 1

ENTRYPOINT ["./scuffed-site-server"]
