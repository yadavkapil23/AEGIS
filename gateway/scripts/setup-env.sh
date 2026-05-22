#!/bin/bash
# AEGIS Gateway - Environment Setup Script

set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_step() { echo -e "${BLUE}→${NC} $1"; }
log_success() { echo -e "${GREEN}✓${NC} $1"; }
log_warn() { echo -e "${YELLOW}!${NC} $1"; }

echo "AEGIS Gateway - Environment Setup"
echo "=================================="
echo ""

# Check system requirements
log_step "Checking system requirements..."

# Rust
if ! command -v cargo &> /dev/null; then
    log_warn "Rust not found. Installing rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
fi
log_success "Rust installed: $(rustc --version)"

# Docker
if ! command -v docker &> /dev/null; then
    log_warn "Docker not found. Please install from https://docker.com"
    exit 1
fi
log_success "Docker installed: $(docker --version)"

# Docker Compose
if ! command -v docker-compose &> /dev/null; then
    log_warn "Docker Compose not found. Installing..."
    curl -L "https://github.com/docker/compose/releases/latest/download/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
    chmod +x /usr/local/bin/docker-compose
fi
log_success "Docker Compose installed: $(docker-compose --version)"

echo ""
log_step "Creating .env file..."

# Create .env if it doesn't exist
if [ ! -f .env ]; then
    cat > .env << 'ENVEOF'
# AEGIS Gateway Environment Configuration

# Security
JWT_SECRET=change-me-in-production-use-strong-key
API_KEYS=sk-demo123,sk-prod456,sk-staging789

# Gateway Configuration
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=8080
GATEWAY_LOG_LEVEL=info

# Backend Configuration
VLLM_ENDPOINTS=http://localhost:8000
LLAMACPP_ENDPOINT=http://localhost:8001

# Rate Limiting
RATE_LIMIT_RPS=100

# Circuit Breaker
CIRCUIT_BREAKER_THRESHOLD=5

# Cache
GATEWAY_CACHE_SIZE=1000

# Timeout
GATEWAY_TIMEOUT=30

# Database
DATABASE_URL=postgres://postgres:password@localhost:5432/aegis

# Observability
RUST_LOG=info
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317
ENVEOF
    log_success "Created .env file"
else
    log_warn ".env already exists (skipping)"
fi

echo ""
log_step "Creating directories..."

mkdir -p \
    /var/backups/aegis/{prometheus,grafana,database} \
    /var/log/aegis \
    logs

log_success "Directories created"

echo ""
log_step "Initializing git hooks..."

if [ -d .git ]; then
    mkdir -p .git/hooks
    cat > .git/hooks/pre-commit << 'HOOKEOF'
#!/bin/bash
# Pre-commit hook: check formatting and tests

set -e

echo "→ Running cargo fmt check..."
cargo fmt -- --check

echo "→ Running cargo clippy..."
cargo clippy -- -D warnings

echo "✓ Pre-commit checks passed"
HOOKEOF
    chmod +x .git/hooks/pre-commit
    log_success "Git hooks installed"
fi

echo ""
log_step "Setting up shell aliases (optional)..."

cat > .aliases << 'ALIASEOF'
# AEGIS Gateway useful aliases

alias gateway-build="cargo build --release"
alias gateway-run="./target/release/gateway"
alias gateway-test="cargo test && ./scripts/test.sh"
alias gateway-clean="cargo clean && docker system prune -f"
alias gateway-logs="docker-compose -f docker-compose.observability.yml logs -f"
alias gateway-metrics="curl http://localhost:8080/metrics"
alias gateway-health="curl http://localhost:8080/health/live"
alias gateway-validate="./scripts/validate.sh"
ALIASEOF

log_success "Created .aliases (source with: source .aliases)"

echo ""
echo "Setup complete!"
echo ""
echo "Next steps:"
echo "  1. Edit .env file (set JWT_SECRET to strong value)"
echo "  2. source .aliases (optional, for quick commands)"
echo "  3. cargo build --release"
echo "  4. docker-compose -f docker-compose.observability.yml up -d"
echo "  5. ./target/release/gateway"
echo ""
echo "See QUICKSTART.md for more details."
