# ==========================================
# DCABOT — Multi-Stage Production Dockerfile
# Stage 1: Rust Core Engine Builder
# Stage 2: Next.js Frontend Builder
# Stage 3: Minimal Production Runtime
# ==========================================

# --- Stage 1: Rust Engine ---
FROM rust:1.85-slim-bookworm AS rust-builder
WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    sqlite3 \
    libsqlite3-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY mini-services/dca-core /build/mini-services/dca-core
WORKDIR /build/mini-services/dca-core
RUN cargo build --release --bin dca_server

# --- Stage 2: Next.js Web UI ---
FROM node:22-bookworm-slim AS node-builder
WORKDIR /app

COPY package*.json ./
RUN npm ci

COPY . .
ENV NODE_ENV=production
RUN npm run build

# --- Stage 3: Production Runner ---
FROM debian:bookworm-slim AS runner
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

ENV NODE_ENV=production
ENV DCA_LOCAL_RUN=0
ENV PORT=3000
ENV HOSTNAME="0.0.0.0"

# Directories for persistence
RUN mkdir -p /app/data /app/logs

# Copy Rust Engine
COPY --from=rust-builder /build/mini-services/dca-core/target/release/dca_server /app/dca_server
RUN chmod +x /app/dca_server

# Copy Next.js Standalone Distribution
COPY --from=node-builder /app/.next/standalone /app/
COPY --from=node-builder /app/.next/static /app/.next/standalone/.next/static
COPY --from=node-builder /app/public /app/.next/standalone/public

# Copy Supervisor Entrypoint
COPY scripts/docker-entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

EXPOSE 3000 3002

VOLUME ["/app/data"]

ENTRYPOINT ["/app/entrypoint.sh"]
