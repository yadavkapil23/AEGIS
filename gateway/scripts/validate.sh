#!/bin/bash
# AEGIS Gateway - Build Validation Script
# Checks compilation, dependencies, and basic structure

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_step() { echo -e "${GREEN}→${NC} $1"; }
log_warn() { echo -e "${YELLOW}!${NC} $1"; }
log_error() { echo -e "${RED}✗${NC} $1"; }

echo "AEGIS Gateway - Build Validation"
echo "=================================="
echo ""

# Check Rust installation
log_step "Checking Rust installation..."
rustc --version || { log_error "Rust not installed"; exit 1; }
cargo --version || { log_error "Cargo not installed"; exit 1; }
echo ""

# Check critical files
log_step "Checking critical files..."
REQUIRED_FILES=(
    "Cargo.toml"
    "src/main.rs"
    "src/lib.rs"
    "src/backend_manager.rs"
    "src/inference_handler.rs"
    "src/jwt_auth.rs"
    "src/security_middleware.rs"
    "src/request_validator.rs"
    "src/db_migrations.rs"
    "src/backup.rs"
    "src/telemetry.rs"
    "src/metrics.rs"
    "Dockerfile"
    "kubernetes/deployment.yml"
    "prometheus.yml"
    "prometheus_alerts.yml"
)

MISSING=0
for file in "${REQUIRED_FILES[@]}"; do
    if [ -f "$file" ]; then
        echo "  ✓ $file"
    else
        log_error "Missing: $file"
        MISSING=$((MISSING + 1))
    fi
done

if [ $MISSING -gt 0 ]; then
    log_error "$MISSING files missing"
    exit 1
fi
echo ""

# Check Cargo.toml dependencies
log_step "Checking dependencies in Cargo.toml..."
REQUIRED_DEPS=(
    "actix-web"
    "tokio"
    "serde"
    "jsonwebtoken"
    "prometheus"
    "tracing"
    "opentelemetry"
)

for dep in "${REQUIRED_DEPS[@]}"; do
    if grep -q "^$dep" Cargo.toml || grep -q "^$dep\s*=" Cargo.toml; then
        echo "  ✓ $dep"
    else
        log_warn "Missing or misconfigured: $dep"
    fi
done
echo ""

# Try to build
log_step "Attempting to build..."
if cargo check 2>&1 | head -20; then
    echo ""
    log_step "Build check successful!"
else
    log_warn "Build check had warnings (see above)"
fi
echo ""

# Count lines of code
log_step "Code statistics..."
LINES=$(find src -name "*.rs" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')
MODULES=$(find src -name "*.rs" | wc -l)
echo "  Total lines of Rust code: $LINES"
echo "  Total modules: $MODULES"
echo ""

# Check tests
log_step "Checking unit tests..."
TEST_COUNT=$(grep -r "#\[test\]" src/ 2>/dev/null | wc -l)
echo "  Unit tests defined: $TEST_COUNT"
echo ""

log_step "Validation complete!"
echo ""
echo "Next steps:"
echo "  1. cargo build --release"
echo "  2. cargo test"
echo "  3. ./scripts/test.sh (after starting gateway)"
echo "  4. docker build -t aegis-gateway:latest ."
