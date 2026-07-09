# AEGIS - Project Overview

**Advanced Engine for Generative Inference & Scheduling**

## Executive Summary
AEGIS (v3.0.0) is a production-grade LLM inference gateway and orchestration system written in Rust. It sits between user-facing applications and Large Language Models, providing multi-backend inference routing, distributed KV-cache management, cryptographic audit trails, and enterprise security.

AEGIS is NOT a model wrapper — it is a full orchestration layer with per-backend circuit breakers, bulkhead concurrency control, and automatic fallback across vLLM, llama.cpp, Ollama, and HuggingFace.

## Status
**v3.0.0 — All 13 crates compile with zero errors. 59 tests passing.**

## Core Capabilities

### Multi-Backend Inference Routing
Routes inference requests across 4 backends with automatic fallback:
1. **vLLM** (primary) — High-throughput OpenAI-compatible API
2. **llama.cpp** (fallback) — Native C++ FFI with temperature/top_p sampling
3. **Ollama** (fallback) — OpenAI-compatible API for local containers
4. **HuggingFace** (fallback) — Cloud Inference API

Each backend has its own circuit breaker (Closed→Open→HalfOpen) and the gateway falls back automatically on failure.

### OpenAI-Compatible API
- `POST /infer` — Synchronous text completion
- `POST /infer/stream` — SSE streaming (token-by-token)
- `POST /v1/chat/completions` — Chat completions (messages array format)

### Enterprise Security
- **JWT Authentication** — HMAC-SHA256 signature verification
- **API Key Management** — SHA-256 hashed keys stored in PostgreSQL, with DB-backed validation
- **Rate Limiting** — Per-IP token bucket via security middleware
- **CORS** — Configurable cross-origin resource sharing
- **Security Headers** — CSP, X-Frame-Options, X-XSS-Protection

### Resilience Patterns
- **Circuit Breaker** — Per-backend failure detection with automatic recovery
- **Bulkhead** — Concurrency control (100 max simultaneous requests)
- **Retry** — Exponential backoff with jitter

### Cryptographic Audit Engine
Every inference event is chained into a BLAKE3 hash tree stored in PostgreSQL — mathematically tamper-proof execution logs for HIPAA, SOC2, and GDPR compliance.

### Distributed KV-Cache Management
- Physical block allocation with LRU eviction
- Raft-inspired consensus across a 3-node scheduler cluster
- WAL persistence for crash recovery
- gRPC communication between gateway and scheduler

### Real-time Observability
- **Prometheus** — Counters, histograms, gauges for all inference metrics
- **Grafana** — Pre-built dashboards
- **OpenTelemetry** — Distributed tracing via Jaeger
- **Structured Logging** — JSON-formatted tracing output

## Tech Stack
| Component | Technology |
|-----------|-----------|
| Language | Rust (Edition 2021) |
| Web Framework | Actix-Web 4 |
| Async Runtime | Tokio |
| gRPC | tonic + prost |
| Database | PostgreSQL (sqlx) |
| Hashing | BLAKE3 |
| Auth | HMAC-SHA256 (hmac), JWT (jsonwebtoken) |
| Metrics | Prometheus + Grafana |
| Tracing | OpenTelemetry + Jaeger |
| CORS | actix-cors |
| HTTP Client | reqwest |
| Rate Limiting | Token bucket (security middleware) |

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/infer` | Synchronous LLM inference |
| POST | `/infer/stream` | SSE streaming inference |
| POST | `/v1/chat/completions` | OpenAI-compatible chat completions |
| POST | `/v1/allocate` | KV-cache block allocation |
| POST | `/v1/deallocate` | KV-cache block deallocation |
| GET | `/v1/stats` | Cache statistics |
| GET | `/v1/cluster` | Cluster health |
| GET | `/health` | Deep health check |
| GET | `/health/ready` | Readiness probe |
| GET | `/health/live` | Liveness probe |
| GET | `/health/startup` | Startup probe |
| GET | `/metrics` | Prometheus metrics |
| GET | `/backends/status` | Backend health status |
| POST | `/api/keys` | Create API key |
| GET | `/api/keys` | List API keys |
| DELETE | `/api/keys/{key}` | Revoke API key |

## Project Structure
```
gateway/              API Gateway (Actix-Web, main binary)
scheduler/            Distributed KV-Cache Scheduler (gRPC, Raft consensus)
consensus/            Consensus engine with peer communication (gRPC client)
inference-backends/   Backend abstractions (vLLM, llama.cpp, HuggingFace)
security/             JWT, API keys, rate limiting, TLS
resilience/           Circuit breaker, retry, timeout, degradation
audit/                BLAKE3 hash chain audit engine
safety/               Policy engine (Allow/Deny/Fallback)
observability/        Prometheus metrics, health probes, tracing
telemetry/            OpenTelemetry integration, OTLP export
proto/                Shared protobuf definitions (inference, scheduling, audit)
runtime/              Top-level orchestrator (wires all subsystems)
benchmarks/           Criterion benchmarks
```

## Target Audience
* **AI Startups** hosting open-source models (Llama 3, Mistral) without per-token API fees
* **Enterprise Infrastructure Teams** requiring compliance, zero-downtime, and tamper-proof audit logs
* **Autonomous Agent Developers** needing fast inference with streaming and fallback routing
