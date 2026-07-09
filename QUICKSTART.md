# AEGIS v3.0.0 - Quickstart Guide

Get the AEGIS LLM Gateway running on your local machine.

## Prerequisites

| Requirement | Version | Notes |
|-------------|---------|-------|
| Rust | 1.75+ | `rustup default stable` |
| Docker | 24+ | For PostgreSQL, Prometheus, Grafana |
| CMake | 3.24+ | Only needed if building llama.cpp backend |
| LLVM/Clang | 17+ | Only needed if building llama.cpp backend |

**Linux/macOS**: Full support out of the box.

**Windows**: Gateway compiles without C++ tools. llama.cpp backend requires LLVM + CMake in PATH.

## Step 1: Start Infrastructure

```bash
docker-compose -f docker-compose-services.yml up -d
```

This starts:
- **PostgreSQL** on `localhost:5432` (API keys, inference logs, audit trail)
- **Prometheus** on `localhost:9090` (metrics collection)
- **Grafana** on `localhost:3000` (dashboards)

## Step 2: Configure Environment

Create a `.env` file or export variables:

```bash
# Required
DATABASE_URL=postgres://postgres:postgres@localhost:5432/aegis
JWT_SECRET=your-secret-key-here

# Inference backends (at least one should be running)
VLLM_ENDPOINT=http://localhost:8000
LLAMACPP_ENDPOINT=http://localhost:8001
OLLAMA_ENDPOINT=http://localhost:11434
HUGGINGFACE_API_KEY=hf_xxxxx  # optional

# Gateway settings
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=8080
RATE_LIMIT_RPS=100
GATEWAY_TIMEOUT=30
API_KEYS=sk-demo123,sk-demo456
```

### Windows PowerShell

```powershell
$env:DATABASE_URL="postgres://postgres:postgres@localhost:5432/aegis"
$env:JWT_SECRET="your-secret-key"
$env:VLLM_ENDPOINT="http://localhost:8000"
```

## Step 3: Start the Gateway

```bash
cargo run --release -p aegis-gateway
```

First build takes 5-10 minutes if llama.cpp is enabled. You'll see:

```
AEGIS Gateway starting on http://0.0.0.0:8080
vLLM endpoint: http://localhost:8000
llama.cpp endpoint: http://localhost:8001
```

## Step 4: Test the API

### Synchronous Inference

```bash
curl -X POST http://localhost:8080/infer \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-demo123" \
  -d '{
    "model": "qwen2.5:0.5b",
    "prompt": "Write a high performance Rust function.",
    "max_tokens": 100,
    "temperature": 0.7,
    "top_p": 0.9
  }'
```

### Streaming Inference (SSE)

```bash
curl -N -X POST http://localhost:8080/infer/stream \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-demo123" \
  -d '{
    "model": "qwen2.5:0.5b",
    "prompt": "Tell me a story",
    "max_tokens": 200
  }'
```

### Chat Completions (OpenAI-Compatible)

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-demo123" \
  -d '{
    "model": "qwen2.5:0.5b",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "What is Rust?"}
    ],
    "max_tokens": 200,
    "temperature": 0.7
  }'
```

### Health Checks

```bash
# Liveness (always returns 200)
curl http://localhost:8080/health/live

# Readiness (200 if at least one backend is up)
curl http://localhost:8080/health/ready

# Deep health (all subsystems)
curl http://localhost:8080/health

# Backend status
curl http://localhost:8080/backends/status
```

### API Key Management

```bash
# Create a new API key
curl -X POST http://localhost:8080/api/keys \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-demo123" \
  -d '{"name": "my-app"}'

# List all keys (masked)
curl http://localhost:8080/api/keys \
  -H "X-API-Key: sk-demo123"

# Revoke a key
curl -X DELETE http://localhost:8080/api/keys/KEY_ID \
  -H "X-API-Key: sk-demo123"
```

### Prometheus Metrics

```bash
curl http://localhost:8080/metrics
```

## Step 5: View Dashboards

| Service | URL | Description |
|---------|-----|-------------|
| Grafana | http://localhost:3000 | Dashboards (admin/admin) |
| Prometheus | http://localhost:9090 | Raw metrics |
| Gateway | http://localhost:8080 | API endpoints |

## Architecture at a Glance

```
Client → Gateway (8080)
           ├── Auth (JWT / API Key)
           ├── Rate Limiting
           ├── CORS
           ├── Circuit Breaker
           ├── Bulkhead
           │
           ├── POST /infer ─────────→ vLLM → llama.cpp → Ollama → HuggingFace
           ├── POST /infer/stream ──→ vLLM (SSE streaming)
           ├── POST /v1/chat/... ───→ vLLM → llama.cpp → Ollama → HuggingFace
           │
           ├── KV-Cache ──gRPC──→ Scheduler (3-node cluster)
           ├── Audit ───────────→ PostgreSQL (BLAKE3 hash chain)
           └── Metrics ─────────→ Prometheus → Grafana
```

## Running Tests

```bash
# All tests
cargo test --workspace

# Specific crates
cargo test -p resilience       # 8 tests
cargo test -p observability    # 10 tests
cargo test -p aegis-gateway    # 41 tests

# Integration tests (requires running infrastructure)
cargo test -p aegis-gateway --test http_endpoint_tests
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `DATABASE_URL` not set | Export `DATABASE_URL=postgres://postgres:postgres@localhost:5432/aegis` |
| No backends available | Start at least one: vLLM, llama.cpp, Ollama, or set HUGGINGFACE_API_KEY |
| Windows: llama.cpp won't build | Install LLVM 17+ and set `$env:LIBCLANG_PATH="C:\Program Files\LLVM\bin"` |
| Port 8080 in use | Set `GATEWAY_PORT=8081` or stop the conflicting process |
| Auth failed | Use `X-API-Key: sk-demo123` header (or create a key via `/api/keys`) |
