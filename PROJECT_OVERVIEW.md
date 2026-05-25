# AEGIS - Project Overview

**Advanced Engine for Generative Inference & Scheduling**

## Executive Summary
AEGIS (v3.0.0) is a production-grade, highly-optimized AI Infrastructure engine written in Rust. It is designed to sit between user-facing applications and Large Language Models, providing extreme performance optimizations, physical memory management, and enterprise-grade security.

It is NOT a simple wrapper around OpenAI's API. AEGIS is a **bare-metal orchestrator** that compiles and executes LLMs natively via C++ FFI.

## Why AEGIS?
Modern AI applications suffer from three major bottlenecks: Latency, VRAM costs, and Security. AEGIS solves all three:
1. **Latency**: By implementing **Speculative Decoding**, AEGIS generates text up to 3x faster by using a small draft model to predict tokens for a large target model.
2. **VRAM Costs**: Through **Physical KV-Cache Management**, AEGIS reuses memory blocks across different API requests (e.g., shared system prompts), allowing more concurrent users on a single GPU.
3. **Security**: With its **Cryptographic Audit Engine**, every AI interaction is hashed into an immutable Merkle Tree inside PostgreSQL, providing undeniable proof of AI behavior for regulated industries (Healthcare, Finance).

## Tech Stack
* **Core Language**: Rust (Actix-Web, Tokio)
* **Supported AI Engines**:
  * **vLLM**: Configured as the primary high-throughput backend.
  * **Native C++**: `llama.cpp` via `llama-cpp-2` FFI bindings for low-latency physical control.
  * **Ollama & HuggingFace**: Built-in HTTP fallback routing for local containers and cloud APIs.
* **Database**: PostgreSQL (sqlx)
* **Telemetry**: Prometheus, Grafana, OpenTelemetry
* **Caching**: Redis (For Distributed Rate Limiting)

## Target Audience
AEGIS is built for:
* **AI Startups** looking to host their own open-source models (Llama 3, Mistral) without paying per-token API fees.
* **Enterprise Infrastructure Teams** that require strict compliance, zero-downtime, and tamper-proof audit logs for AI execution.
* **Autonomous Agent Developers** who need sub-millisecond latency and ultra-fast speculative token generation for Agentic reasoning loops.
