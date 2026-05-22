#!/bin/bash
# AEGIS Gateway Deployment Script
# Builds, tests, and deploys gateway to production

set -e

REGISTRY="${DOCKER_REGISTRY:-registry.aegis.ai}"
IMAGE_NAME="aegis-gateway"
VERSION="${VERSION:-latest}"
ENVIRONMENT="${ENVIRONMENT:-staging}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Step 1: Build
log_info "Step 1: Building Docker image..."
docker build \
    --build-arg RUST_VERSION=latest \
    -t ${REGISTRY}/${IMAGE_NAME}:${VERSION} \
    -t ${REGISTRY}/${IMAGE_NAME}:latest \
    -f Dockerfile \
    .

if [ $? -ne 0 ]; then
    log_error "Docker build failed"
    exit 1
fi

log_info "Docker image built successfully"

# Step 2: Test (optional)
if [ "$SKIP_TESTS" != "true" ]; then
    log_info "Step 2: Running tests..."
    docker run --rm ${REGISTRY}/${IMAGE_NAME}:${VERSION} cargo test
    if [ $? -ne 0 ]; then
        log_error "Tests failed"
        exit 1
    fi
    log_info "Tests passed"
fi

# Step 3: Push to registry
log_info "Step 3: Pushing image to registry..."
docker push ${REGISTRY}/${IMAGE_NAME}:${VERSION}
docker push ${REGISTRY}/${IMAGE_NAME}:latest

if [ $? -ne 0 ]; then
    log_error "Docker push failed"
    exit 1
fi

log_info "Image pushed successfully"

# Step 4: Deploy to Kubernetes
log_info "Step 4: Deploying to Kubernetes (${ENVIRONMENT})..."

# Update image in deployment manifest
sed -i "s|image: aegis-gateway:.*|image: ${REGISTRY}/${IMAGE_NAME}:${VERSION}|g" \
    kubernetes/deployment.yml

# Apply manifests
kubectl apply -f kubernetes/deployment.yml

# Wait for rollout
kubectl rollout status deployment/aegis-gateway -n aegis-gateway --timeout=5m

if [ $? -ne 0 ]; then
    log_error "Kubernetes deployment failed"
    exit 1
fi

log_info "Kubernetes deployment successful"

# Step 5: Health checks
log_info "Step 5: Running health checks..."

# Wait for service to be ready
sleep 10

GATEWAY_IP=$(kubectl get service aegis-gateway -n aegis-gateway -o jsonpath='{.status.loadBalancer.ingress[0].ip}')
if [ -z "$GATEWAY_IP" ]; then
    GATEWAY_IP=$(kubectl get service aegis-gateway -n aegis-gateway -o jsonpath='{.spec.clusterIP}')
fi

# Check liveness
curl -f http://${GATEWAY_IP}:8080/health/live > /dev/null
if [ $? -ne 0 ]; then
    log_error "Liveness check failed"
    exit 1
fi

# Check readiness
curl -f http://${GATEWAY_IP}:8080/health/ready > /dev/null
if [ $? -ne 0 ]; then
    log_error "Readiness check failed"
    exit 1
fi

log_info "Health checks passed"

# Step 6: Smoke test
log_info "Step 6: Running smoke test..."

API_KEY="sk-demo123"
RESPONSE=$(curl -s -X POST http://${GATEWAY_IP}:8080/infer \
    -H "Content-Type: application/json" \
    -H "X-API-Key: ${API_KEY}" \
    -d '{
        "model": "llama-7b",
        "prompt": "What is AI?",
        "max_tokens": 100
    }')

if echo "$RESPONSE" | grep -q "success"; then
    log_info "Smoke test passed"
else
    log_warn "Smoke test response: $RESPONSE"
fi

# Step 7: Update DNS/Load Balancer (optional)
if [ "$UPDATE_DNS" == "true" ]; then
    log_info "Step 7: Updating DNS/Load Balancer..."
    ./scripts/update-dns.sh ${ENVIRONMENT} ${GATEWAY_IP}
fi

log_info "Deployment complete!"
log_info "Gateway is running at http://${GATEWAY_IP}:8080"
log_info "Metrics available at http://${GATEWAY_IP}:8080/metrics"

echo ""
echo "Next steps:"
echo "  - Monitor logs: kubectl logs -f deployment/aegis-gateway -n aegis-gateway"
echo "  - Check metrics: http://prometheus:9090"
echo "  - View dashboards: http://grafana:3000"
echo "  - Run tests: ./scripts/test.sh"
