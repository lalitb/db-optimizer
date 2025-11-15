# Multi-stage Dockerfile for db-optimizer
# Production-ready build with minimal runtime footprint

# Stage 1: Builder
FROM rust:1.75-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    clang \
    llvm \
    libelf-dev \
    linux-headers-generic \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy dependency manifests first for better caching
COPY Cargo.toml Cargo.lock ./

# Create dummy main.rs to cache dependencies
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy actual source code
COPY src ./src
COPY bpf ./bpf

# Build the actual application
RUN cargo build --release && \
    strip /build/target/release/db-optimizer

# Stage 2: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 dbopt && \
    mkdir -p /data /config && \
    chown -R dbopt:dbopt /data /config

WORKDIR /app

# Copy binary from builder
COPY --from=builder /build/target/release/db-optimizer /usr/local/bin/db-optimizer

# Copy example config
COPY config.example.yml /config/config.example.yml

# Set ownership
RUN chown -R dbopt:dbopt /app

USER dbopt

# Volume for persistent data
VOLUME ["/data", "/config"]

# Expose potential HTTP API port (if implemented)
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD db-optimizer --version || exit 1

# Default command (can be overridden)
ENTRYPOINT ["db-optimizer"]
CMD ["--help"]
