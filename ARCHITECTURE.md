# AEGIS v3.0.0 - Systems Architecture

This document outlines the deeply optimized, native architecture of the AEGIS Gateway and Inference Engine.

## 1. Native C++ Integration (The Core Engine)
Instead of relying on slow HTTP wrappers, AEGIS embeds the `llama.cpp` inference engine directly into the Rust runtime via C++ Foreign Function Interface (FFI) using the `llama-cpp-2` bindings. 
* **Zero Network Overhead**: Tensors and memory blocks are passed between Rust and the GPU directly in physical memory.
* **Fallback Mechanisms**: While native `llama.cpp` handles primary inference, the backend manager can seamlessly fallback to external APIs (vLLM, Ollama) if the native GPU runs out of VRAM.

## 2. Speculative Decoding Pipeline
To achieve extreme token generation speeds, AEGIS implements Speculative Decoding:
1. **Drafting Phase**: A very small, fast LLM generates a "draft" sequence of 4-5 tokens instantly.
2. **Verification Phase**: The massive target LLM processes the draft sequence in a single parallel batch.
3. **Acceptance**: Valid tokens are kept, and if the draft diverges, the target model corrects it. This results in 2x-3x faster inference without degrading output quality.

## 3. Physical KV-Cache Management
AEGIS completely controls the physical memory allocation of the LLM context window (the KV-Cache).
* **Paged Attention**: Memory is broken into fixed-size blocks (e.g., 16 tokens per block).
* **Zero-Copy Routing**: When a user sends a prompt that shares a prefix with a previous prompt (like a shared system prompt), AEGIS routes the physical memory pointers to the new request, entirely bypassing re-computation.
* **LRU Eviction**: Rust-level eviction algorithms instantly clear unused blocks when VRAM hits 90% capacity.

## 4. Cryptographic Merkle Audit Engine
For enterprise compliance (HIPAA, SOC2), AEGIS secures every inference request.
* **Hash Chaining**: Every request and response is hashed using SHA-256.
* **Merkle Trees**: The hash of Request N includes the hash of Request N-1. If anyone alters a log in the PostgreSQL database, the entire cryptographic chain breaks, instantly flagging tampering.

## 5. The API Gateway Layer (Actix-Web)
Before a request ever touches the GPU, it must pass through the high-performance Rust Gateway:
* **Authentication**: JWT validation and API Key checking via PostgreSQL.
* **Token Bucket Rate Limiting**: Prevents abuse and GPU starvation.
* **Observability**: Every layer emits OpenTelemetry traces, visualized in Grafana.
