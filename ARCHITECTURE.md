# AEGIS v3.0.0 - Systems Architecture

This document describes the architecture of the AEGIS LLM Gateway and Inference Orchestration System.

---

## 1. System Overview

AEGIS is a 13-crate Rust workspace. The gateway crate is the main binary; all other crates are libraries consumed by it and by each other.

```
                    ┌──────────────────────────────┐
                    │        Client Request         │
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────▼───────────────┐
                    │     AEGIS Gateway (8080)      │
                    │     Actix-Web HTTP Server      │
                    └──────────────┬───────────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │                    │                    │
    ┌─────────▼─────────┐ ┌───────▼───────┐ ┌─────────▼─────────┐
    │  Inference Handler │ │  API Key CRUD │ │  Allocation Proxy  │
    │  POST /infer       │ │  POST/GET/DEL │ │  POST /v1/allocate │
    │  POST /infer/stream│ │  /api/keys    │ │  GET  /v1/stats    │
    └─────────┬─────────┘ └───────────────┘ └─────────┬─────────┘
              │                                        │
    ┌─────────▼─────────┐                    ┌─────────▼─────────┐
    │   LLM Backend     │                    │  AllocationClient  │
    │   (4 backends)    │                    │  (tonic gRPC)      │
    └─────────┬─────────┘                    └─────────┬─────────┘
              │                                        │
    ┌─────────▼─────────┐                    ┌─────────▼─────────┐
    │ Backend Manager   │                    │ KV-Cache Scheduler │
    │ Circuit Breaker   │                    │ (3-node cluster)   │
    │ Bulkhead          │                    └─────────┬─────────┘
    └───────────────────┘                              │
                                              ┌───────▼───────┐
                                              │ Raft Consensus │
                                              │ Block Allocator│
                                              │ WAL Persist    │
                                              └───────────────┘
```

---

## 2. Request Flow

### 2.1 Synchronous Inference (`POST /infer`)

```
1. Client sends HTTP POST with JSON body and auth header
2. Middleware stack processes request:
   a. RequestIdMiddleware   → assigns UUID-4
   b. Logger                → structured request logging
   c. SecurityHeadersMiddleware → CSP, X-Frame-Options
   d. RateLimitMiddleware   → token bucket check (global/per-key/per-IP)
   e. JwtAuthMiddleware     → validates JWT or API key against PostgreSQL
3. InferenceHandler validates request (model, prompt, max_tokens, temperature, top_p)
4. LLMBackend.infer() tries backends in order:
   a. vLLM (primary)       → HTTP POST to /v1/completions
   b. llama.cpp (fallback)  → HTTP POST to /completion
   c. Ollama (fallback)     → HTTP POST to /v1/completions
   d. HuggingFace (fallback)→ HTTP POST to /models/{model}
   Each backend has its own circuit breaker (Closed→Open→HalfOpen)
5. On success: record Prometheus metrics, log to PostgreSQL async, return JSON
6. On failure: record error metric, log failure async, return 500
```

### 2.2 Streaming Inference (`POST /infer/stream`)

```
1. Same validation as synchronous path
2. Gateway sends streaming request to vLLM with "stream": true
3. vLLM returns newline-delimited JSON chunks (SSE format)
4. Gateway forwards each chunk as Server-Sent Events to the client
5. Client receives tokens in real-time as they are generated
```

### 2.3 KV-Cache Allocation (`POST /v1/allocate`)

```
1. Client sends allocation request (request_id, num_blocks, owner)
2. Gateway proxy forwards to scheduler via tonic gRPC
3. Scheduler allocates blocks from the KV-cache allocator
4. Returns allocated block IDs, latency, node ID
```

---

## 3. Core Components

### 3.1 Gateway (`gateway/`)

The main binary. An Actix-Web HTTP server with 6 middleware layers and 14 endpoints.

| File | Purpose |
|------|---------|
| `main.rs` | Server bootstrap, middleware wiring, service initialization |
| `middleware.rs` | `GatewayState` — central shared state (allocation_client, cache, config, db, backends, metrics, queue) |
| `inference_handler.rs` | `POST /infer`, `POST /infer/stream`, health probes, metrics endpoint |
| `handlers.rs` | `POST /v1/allocate`, `GET /v1/stats`, `GET /v1/cluster` — proxy to scheduler gRPC |
| `api_key_handlers.rs` | `POST/GET/DELETE /api/keys` — CRUD via PostgreSQL |
| `service.rs` | `InferenceService` — orchestrates validate→queue→circuit-check→infer→metrics→DB-log |
| `llm_backend.rs` | `LLMBackend` — 4-backend client with per-backend circuit breakers and retry |
| `backend_manager.rs` | `BackendManager` — circuit breaker state machine, bulkhead concurrency, latency tracking |
| `request_validator.rs` | Request validation rules (model, prompt, tokens, temperature, top_p) |
| `request_queue.rs` | Async FIFO queue with timeout, RAII permit, and wake-up notification |
| `allocation_client.rs` | Tonic gRPC client to scheduler cluster with health-aware node selection |
| `database.rs` | PostgreSQL pool, API key CRUD, inference logging, audit logging |
| `db_migrations.rs` | Schema migrations with tracking table and rollback support |
| `backup.rs` | pg_dump database backup, Prometheus snapshot, retention cleanup |
| `metrics.rs` | Prometheus counters/histograms/gauges for all inference metrics |
| `build.rs` | Compiles `scheduler/proto/allocation.proto` for gRPC client stubs |

### 3.2 Scheduler (`scheduler/`)

Distributed KV-cache management with Raft consensus across a 3-node cluster.

| Component | Purpose |
|-----------|---------|
| `allocator.rs` | `KVCacheAllocator` — fixed-block memory management with free-list, fragmentation tracking |
| `consensus.rs` | `QuorumConsensus` — Raft-style quorum voting and leader election |
| `consensus_kv_cache.rs` | Routes allocation through consensus leader |
| `consensus_allocator.rs` | Command log backed by local allocator with ownership state |
| `state_machine.rs` | Applies log entries (Allocate, Deallocate, RegisterPeer) with idempotency |
| `state_machine_replication.rs` | Follower tracking, quorum replication, commit index advancement |
| `replicated_log.rs` | Append-only `VecDeque` log with commit/apply index management |
| `persistence.rs` | Write-Ahead Log (WAL) file I/O with crash recovery and snapshot compaction |
| `distributed.rs` | `DistributedKVCache` — multi-node allocation with local-first, remote-fallback |
| `block_ownership.rs` | Bidirectional `DashMap` tracking block↔node ownership with timestamps |
| `failure_detector.rs` | Heartbeat-based dead node detection with recovery tracking |
| `consistency.rs` | BLAKE3 state hash validation, double-ownership detection, drift detection |
| `grpc_server.rs` | Tonic `SchedulingService` implementation |
| `node_selector.rs` | Weighted scoring: capacity 50%, latency 30%, load 20% |

### 3.3 Consensus (`consensus/`)

Standalone consensus engine with gRPC peer communication.

| Component | Purpose |
|-----------|---------|
| `lib.rs` | `ConsensusEngine` — Raft state machine (Follower/Candidate/Leader), election, heartbeat |
| `log.rs` | `ReplicatedLog` — append-only log with term tracking |
| `state.rs` | `ExecutionState` — key-value state with snapshot support |
| `peer_client.rs` | `PeerClient` — tonic gRPC client for RequestVote and AppendEntries RPCs |
| `proto/consensus.proto` | Protobuf definitions for consensus RPCs |

### 3.4 Inference Backends (`inference-backends/`)

Abstraction layer for multiple AI inference engines.

| Component | Purpose |
|-----------|---------|
| `traits.rs` | `InferenceBackend` async trait: `infer()`, `health_check()`, `supports_model()`, `get_models()` |
| `router.rs` | `BackendRouter` — health-aware fallback routing with preference selection |
| `vllm.rs` | vLLM client with round-robin, least-loaded, and random endpoint selection |
| `llamacpp.rs` | llama.cpp HTTP backend implementing `InferenceBackend` trait |
| `llama_cpp_safe.rs` | Safe Rust FFI wrapper: `Model`, `Context`, `Session` with temperature/top_p sampling |
| `llama_cpp_sys.rs` | Raw unsafe FFI bindings to llama.cpp C library |
| `huggingface.rs` | HuggingFace Inference API backend |
| `production_manager.rs` | `CircuitBreaker`, `RateLimiter`, `Bulkhead`, retry with exponential backoff |
| `mock.rs` | Mock backend for testing (generates fake tokens from word list) |

### 3.5 Security (`security/`)

| Component | Purpose |
|-----------|---------|
| `auth.rs` | `AuthenticationProvider` trait, `MultiAuthProvider`, `Principal` with permission checks |
| `apikey.rs` | API key generation (`sk-` + 32 hex), SHA-256 hashing, revocation, rotation |
| `jwt.rs` | JWT creation/validation via `jsonwebtoken`, with expiration and clock skew handling |
| `rate_limiter.rs` | Three-tier token bucket: global (10k RPS), per-key (1k RPS), per-IP (100 RPS) |
| `tls.rs` | TLS/mTLS configuration via `rustls`, certificate validation |

### 3.6 Resilience (`resilience/`)

| Component | Purpose |
|-----------|---------|
| `circuit_breaker.rs` | Three-state breaker (Closed→Open→HalfOpen) with configurable thresholds and timeout recovery |
| `retry.rs` | Exponential backoff with jitter via `tokio::time::sleep` |
| `timeout.rs` | Generic timeout wrapper around any `Future` |
| `graceful_degradation.rs` | Degradation levels with primary/fallback execution paths |

### 3.7 Audit (`audit/`)

| Component | Purpose |
|-----------|---------|
| `engine.rs` | `AuditEngine` — BLAKE3 event hashing, append to trail, integrity verification |
| `trail.rs` | `ExecutionTrail` — append-only hash chain (each event includes previous hash) |
| `metrics.rs` | Event count and hash computation latency tracking |

### 3.8 Observability (`observability/`)

| Component | Purpose |
|-----------|---------|
| `metrics.rs` | `MetricsRegistry` — Prometheus counters, gauges, histograms for all subsystems |
| `health.rs` | `HealthManager` — liveness/readiness probes with component-level checks |
| `tracing.rs` — `tracing-subscriber` initialization with env filter and JSON output |

### 3.9 Telemetry (`telemetry/`)

| Component | Purpose |
|-----------|---------|
| `metrics.rs` | Global Prometheus metrics via `OnceLock` (9 metrics: requests, errors, tokens, latency, cache hits/misses) |
| `distributed_tracing.rs` | `DistributedTraceContext`, `SpanRecorder`, `TracingMetrics` |
| `otlp_export.rs` | OTLP pipeline setup for Jaeger export |

### 3.10 Other Crates

| Crate | Purpose |
|-------|---------|
| `safety/` | Policy DSL: Allow/Deny/Fallback rules evaluated on inference requests |
| `proto/` | Shared protobuf definitions: `inference.proto`, `scheduling.proto`, `audit.proto` |
| `runtime/` | Top-level orchestrator wiring all subsystems (scheduler, safety, audit, consensus) |
| `benchmarks/` | Criterion benchmarks for allocation, audit, and end-to-end inference |

---

## 4. Key Algorithms

### 4.1 Circuit Breaker State Machine

```
        success          failure
    ┌─────────┐     ┌─────────┐
    │         │     │         │
    ▼         │     ▼         │
 ┌──────┐  failure  ┌──────┐  timeout  ┌──────────┐
 │CLOSED│──────────▶│ OPEN │──────────▶│ HALF-OPEN│
 └──────┘           └──────┘           └──────────┘
    ▲                                     │
    │            success × threshold      │
    └─────────────────────────────────────┘
```

- **Closed**: Normal operation. Failures counted. Opens when failure rate exceeds threshold.
- **Open**: All requests rejected. Timer starts. After timeout, transitions to HalfOpen.
- **HalfOpen**: One test request allowed. Success → Closed. Failure → Open.

### 4.2 KV-Cache Allocation

```
Free List: [4, 7, 12, 15, 20, ...]
                    │
    allocate(3)     │
                    ▼
Allocated: [4, 7, 12]    Free List: [15, 20, ...]
                    │
    deallocate([4]) │
                    ▼
Allocated: [7, 12]        Free List: [4, 15, 20, ...]
```

- Fixed block size (16 tokens per block)
- DashMap for O(1) block lookup
- Physical eviction via `llama_kv_cache_rm()` when bound to a llama.cpp session
- Fragmentation ratio tracked: `free_blocks / total_blocks`

### 4.3 Raft Consensus

```
Node A (Leader)          Node B (Follower)       Node C (Follower)
     │                        │                        │
     │──── AppendEntries ────▶│                        │
     │──── AppendEntries ─────────────────────────────▶│
     │◀─── Ack ──────────────│                        │
     │◀─── Ack ──────────────────────────────────────│
     │                        │                        │
     │ (commit when majority ack)                     │
```

- Leader election via RequestVote RPCs
- Heartbeat via AppendEntries with empty entries
- Log replication with prev_log_index/prev_log_term consistency check
- WAL persistence for crash recovery
- BLAKE3 state hash for cross-node consistency validation

### 4.4 Temperature/Top-P Sampling

```
Logits: [2.1, 0.5, -1.2, 3.8, 0.1]
            │
    Temperature = 0.7
    scaled = logits / 0.7
            │
    Softmax → probabilities
            │
    Top-P = 0.9
    Sort by probability, accumulate until ≥ 0.9
    Filter to nucleus tokens
            │
    Renormalize, sample via rand
            │
    Selected token
```

---

## 5. Data Flow

### 5.1 Request → Response

```
HTTP Request
  → Actix-Web middleware stack (6 layers)
  → InferenceHandler::validate()
  → LLMBackend::infer()
    → try vLLM (circuit breaker check → HTTP POST → parse response)
    → fallback to llama.cpp (circuit breaker check → HTTP POST → parse response)
    → fallback to Ollama (circuit breaker check → HTTP POST → parse response)
    → fallback to HuggingFace (circuit breaker check → HTTP POST → parse response)
  → Record Prometheus metrics
  → Log to PostgreSQL (async tokio::spawn)
  → HTTP Response
```

### 5.2 KV-Cache Lifecycle

```
Request arrives
  → Scheduler allocates N blocks from free list
  → Blocks marked as owned by request_id
  → Physical blocks bound to llama.cpp session (optional)
  → Request completes
  → Scheduler deallocates blocks
  → Blocks returned to free list
  → If VRAM > 90%: LRU eviction of least-recently-used blocks
```

### 5.3 Audit Trail

```
Event occurs (e.g., TOKEN_GENERATED)
  → Serialize event to JSON bytes
  → BLAKE3 hash = hash(event_bytes)
  → Append to trail: { event, hash, prev_hash }
  → Each hash includes the previous hash (chain)
  → Store in PostgreSQL
  → Verification: recompute chain, check all hashes match
```

---

## 6. Deployment Topology

### 6.1 Single Node (Development)

```
┌─────────────────────────────────┐
│           Docker Host           │
│                                 │
│  ┌─────────┐  ┌──────────────┐ │
│  │ Gateway  │  │ PostgreSQL   │ │
│  │ :8080   │  │ :5432        │ │
│  └─────────┘  └──────────────┘ │
│  ┌─────────┐  ┌──────────────┐ │
│  │Prometheus│  │   Grafana    │ │
│  │ :9090   │  │   :3000      │ │
│  └─────────┘  └──────────────┘ │
└─────────────────────────────────┘
```

### 6.2 Production Cluster (3 Nodes)

```
┌──────────────────────────────────────────────────────────────┐
│                     Nginx Load Balancer (:50050)              │
└───────────┬──────────────────┬──────────────────┬────────────┘
            │                  │                  │
   ┌────────▼────────┐ ┌──────▼──────┐ ┌────────▼────────┐
   │  Node 1 (Leader)│ │ Node 2      │ │ Node 3          │
   │  Scheduler      │ │ Scheduler   │ │ Scheduler       │
   │  gRPC :50051    │ │ gRPC :50052 │ │ gRPC :50053     │
   │  Metrics :9001   │ │ Metrics :9002│ │ Metrics :9003  │
   └─────────────────┘ └─────────────┘ └─────────────────┘
            │                  │                  │
   ┌────────▼──────────────────▼──────────────────▼────────────┐
   │              Shared Infrastructure                         │
   │  PostgreSQL    Prometheus    Jaeger    Grafana             │
   └───────────────────────────────────────────────────────────┘
```

---

## 7. Configuration

All configuration is via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `GATEWAY_HOST` | `0.0.0.0` | Gateway bind address |
| `GATEWAY_PORT` | `8080` | Gateway port |
| `VLLM_ENDPOINT` | `http://localhost:8000` | vLLM API endpoint |
| `LLAMACPP_ENDPOINT` | `http://localhost:8001` | llama.cpp API endpoint |
| `OLLAMA_ENDPOINT` | `http://aegis-ollama:11434` | Ollama API endpoint |
| `HUGGINGFACE_API_KEY` | (none) | HuggingFace API key |
| `DATABASE_URL` | `postgres://...` | PostgreSQL connection string |
| `JWT_SECRET` | `change-me-in-production` | JWT signing secret |
| `API_KEYS` | `sk-demo123` | Fallback API keys |
| `RATE_LIMIT_RPS` | `100` | Global rate limit (requests per second) |
| `SCHEDULER_NODES` | `http://localhost:50052` | Comma-separated scheduler gRPC addresses |
| `GATEWAY_CACHE_SIZE` | `1000` | LRU request cache size |
| `GATEWAY_TIMEOUT` | `30` | Request timeout (seconds) |

---

## 8. Error Handling

### 8.1 Backend Failure Cascade

```
vLLM fails → circuit breaker records failure
  → if failures >= 5: circuit opens (rejects requests)
  → try llama.cpp
    → if fails: try Ollama
      → if fails: try HuggingFace
        → if all fail: return 500 "All backends failed"
```

### 8.2 Recovery

```
Circuit breaker in Open state
  → after 30 seconds: transitions to HalfOpen
  → allows ONE test request
  → if success: back to Closed (recovered)
  → if failure: back to Open (still broken)
```

---

## 9. Security Model

### Authentication Flow

```
Request arrives
  → Extract "Authorization: Bearer <jwt>" or "X-API-Key: sk-xxx"
  → If JWT: validate signature, expiry, issuer via jsonwebtoken
  → If API key: SHA-256 hash → lookup in PostgreSQL → check is_active
  → If invalid: return 401
  → If valid: attach Principal to request context
```

### Rate Limiting (3 Tiers)

```
Global:     10,000 RPS (entire gateway)
Per-API-Key: 1,000 RPS (per authenticated user)
Per-IP:       100 RPS (per client IP)
```

Token bucket algorithm with lazy per-identity limiters stored in `DashMap`.

---

## 10. Testing

| Test Type | Location | Count | What It Covers |
|-----------|----------|-------|----------------|
| Integration | `gateway/tests/http_endpoint_tests.rs` | 17 | HTTP health endpoints, request validation |
| Unit | Various `#[cfg(test)]` modules | 50+ | Allocator, circuit breaker, audit trail, consensus |
| Benchmarks | `benchmarks/benches/` | 3 | Allocation throughput, audit latency, e2e inference |

```bash
cargo test --workspace                    # All tests
cargo test -p aegis-gateway --test http_endpoint_tests  # Gateway integration
cargo bench -p aegis-benchmarks           # Performance benchmarks
```
