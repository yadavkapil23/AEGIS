# AEGIS - Credentials & Configuration Reference

## Quick Copy-Paste for Local Development

```bash
# PostgreSQL (docker-compose default)
export DATABASE_URL="postgres://postgres:password@localhost:5433/aegis_gateway"

# JWT Secret (change in production!)
export JWT_SECRET="dev-secret-for-local-testing"

# Ollama (recommended for local inference)
export OLLAMA_ENDPOINT="http://localhost:11434"

# API Keys (required for all requests via X-API-Key header)
export API_KEYS="sk-demo123"

# Logging
export RUST_LOG="info"

# Gateway
export GATEWAY_PORT="8080"
```

Then run:
```bash
cargo run -p aegis-gateway
```

---

## All Available Environment Variables

### Database
- **DATABASE_URL** (required): `postgres://user:password@host:port/database`
  - Example: `postgres://postgres:password@localhost:5433/aegis_gateway`
  - Used for: API keys, audit logs, session storage

### Authentication
- **JWT_SECRET** (required): Random string, min 32 characters
  - Development: `dev-secret-123` or similar
  - Production: Generate with `openssl rand -base64 32`
  - Used for: Signing JWT tokens for session authentication

- **API_KEYS** (default: `sk-demo123`): Comma-separated API keys
  - Format: `sk-key1,sk-key2,sk-key3`
  - Used for: X-API-Key header authentication on all requests

### Inference Backends (choose at least one)
- **OLLAMA_ENDPOINT**: `http://localhost:11434` (recommended for local dev)
- **VLLM_ENDPOINT**: `http://localhost:8000` (high-throughput)
- **LLAMACPP_ENDPOINT**: `http://localhost:8001` (lightweight, CPU-friendly)
- **HUGGINGFACE_API_KEY**: Get from https://huggingface.co/settings/tokens
- **HUGGINGFACE_ENDPOINT** (default): `https://api-inference.huggingface.co/models`

### Gateway Configuration
- **GATEWAY_PORT** (default: `8080`): HTTP listening port
- **GATEWAY_HOST** (default: `0.0.0.0`): Bind address
- **GATEWAY_TIMEOUT** (default: `30`): Request timeout in seconds
- **GATEWAY_CACHE_SIZE** (default: `1000`): Response cache entries
- **RATE_LIMIT_RPS** (default: `100`): Max requests per second

### Scheduler (for distributed setup)
- **SCHEDULER_NODES** (default: `http://localhost:50052`): gRPC endpoints, comma-separated

### Logging & Monitoring
- **RUST_LOG** (default: `info`): Log level — `debug`, `info`, `warn`, `error`

---

## Docker Compose Services Credentials

When running `docker-compose -f docker-compose-services.yml up -d`:

### PostgreSQL
- User: `postgres`
- Password: `password` (from docker-compose file)
- Database: `aegis_gateway`
- Port: `5433` (mapped to 5432 inside container)
- URL: `postgres://postgres:password@localhost:5433/aegis_gateway`

### pgAdmin (Web UI)
- URL: http://localhost:5050
- Email: `admin@aegis.local` (from .env)
- Password: Check `.env` file for `PGADMIN_PASSWORD`

### Grafana (Dashboards)
- URL: http://localhost:3000
- User: `admin` (from .env)
- Password: Check `.env` file for `GRAFANA_PASSWORD`
- Default: `admin` / `admin` (if .env not customized)

### Prometheus (Metrics)
- URL: http://localhost:9090
- No authentication

### Redis (Session Cache)
- Host: `localhost`
- Port: `6379`
- No authentication (default)

---

## Production Checklist

Before deploying to production:

- [ ] Change `JWT_SECRET` to a strong random value: `openssl rand -base64 32`
- [ ] Change `API_KEYS` to unique keys: `openssl rand -hex 16`
- [ ] Change PostgreSQL password in docker-compose file
- [ ] Change pgAdmin and Grafana passwords in .env
- [ ] Use a real `OLLAMA_ENDPOINT` or `VLLM_ENDPOINT` (not localhost)
- [ ] Set `RUST_LOG=warn` (reduce log volume)
- [ ] Set `RATE_LIMIT_RPS` based on your infrastructure capacity
- [ ] Enable HTTPS/TLS (configure in gateway)
- [ ] Set up firewall rules (only expose port 8080 to trusted networks)
- [ ] Configure database backups
- [ ] Monitor Prometheus metrics in Grafana

---

## Example: Complete Setup Script

```bash
#!/bin/bash

# Set all required variables
export DATABASE_URL="postgres://postgres:mypassword@localhost:5433/aegis_gateway"
export JWT_SECRET=$(openssl rand -base64 32)
export API_KEYS="sk-$(openssl rand -hex 16),sk-$(openssl rand -hex 16)"
export OLLAMA_ENDPOINT="http://localhost:11434"
export GATEWAY_PORT="8080"
export RUST_LOG="info"

# Start Docker services
cd /path/to/AEGIS
docker-compose -f docker-compose-services.yml up -d

# Wait for Postgres to be ready
sleep 10

# Run the gateway
cargo run -p aegis-gateway

echo "Gateway running on http://localhost:8080"
echo "API Keys (save these): $API_KEYS"
echo "JWT Secret (keep secret): $JWT_SECRET"
```

---

## Testing the Setup

Once running:

```bash
# Check gateway is up
curl http://localhost:8080/ready

# Test inference with API key
curl -X POST http://localhost:8080/infer \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-demo123" \
  -d '{
    "model": "qwen2.5:0.5b",
    "prompt": "Say hi in 3 words",
    "max_tokens": 20
  }'

# Check Prometheus
curl http://localhost:9090/api/v1/query?query=up

# View Grafana dashboards
# Open: http://localhost:3000 (admin / admin)
```
