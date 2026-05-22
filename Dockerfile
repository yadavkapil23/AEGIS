# Multi-stage Dockerfile for Aegis Scheduler
# Stage 1: Builder
FROM rust:1.75 as builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace
COPY . .

# Build scheduler
RUN cargo build --release -p aegis-scheduler

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /build/target/release/scheduler /app/scheduler

# Create data directory for persistence
RUN mkdir -p /data

# Copy default config
COPY scheduler/config/scheduler.yaml /app/config/scheduler.yaml

# Expose ports
EXPOSE 50051 50052 9090

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/app/scheduler", "health-check"]

# Default environment
ENV SCHEDULER_HOST=0.0.0.0
ENV SCHEDULER_PORT=50051
ENV SCHEDULER_GRPC_PORT=50052
ENV SCHEDULER_METRICS_PORT=9090
ENV SCHEDULER_LOG_LEVEL=info
ENV SCHEDULER_DATA_DIR=/data

# Run scheduler
CMD ["/app/scheduler"]
