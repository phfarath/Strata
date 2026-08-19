# Multi-stage Dockerfile for Strata Cloud Sync Server on Railway
# Build Stage
FROM rust:bookworm AS builder

WORKDIR /app

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/strata-core/Cargo.toml crates/strata-core/
COPY crates/strata-memory/Cargo.toml crates/strata-memory/
COPY crates/strata-tools/Cargo.toml crates/strata-tools/
COPY crates/strata-reasoning/Cargo.toml crates/strata-reasoning/
COPY crates/strata-cli/Cargo.toml crates/strata-cli/
COPY crates/strata-evals/Cargo.toml crates/strata-evals/
COPY crates/strata-server/Cargo.toml crates/strata-server/

# Copy full source tree
COPY crates ./crates

# Build release binary for strata-server
RUN cargo build --release -p strata-server --bin strata-server

# Runtime Stage
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/strata-server /usr/local/bin/strata-server

# Create data directory for SQLite persistence
RUN mkdir -p /data && chmod 777 /data
VOLUME ["/data"]

ENV HOST=0.0.0.0 \
    PORT=8080 \
    DATABASE_PATH=/data/strata_sync.db \
    RUST_LOG=strata_server=info,tower_http=info

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/strata-server"]
