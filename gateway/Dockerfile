# Multi-stage Dockerfile for AEGIS Gateway
# Build stage: Compile Rust application
FROM rust:latest as builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Copy source code
COPY . .

# Build release binary
RUN cargo build --release --bin gateway

# Runtime stage: Minimal production image
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 gateway

# Copy binary from builder
COPY --from=builder /app/target/release/gateway /usr/local/bin/gateway

# Copy config files
COPY gateway/prometheus.yml /etc/prometheus/
COPY gateway/prometheus_alerts.yml /etc/prometheus/
COPY gateway/grafana_dashboard.json /etc/grafana/

# Create directories for backups and logs
RUN mkdir -p /var/backups/aegis /var/log/aegis && \
    chown -R gateway:gateway /var/backups/aegis /var/log/aegis

# Switch to non-root user
USER gateway

# Expose ports
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health/live || exit 1

# Run gateway
CMD ["gateway"]
