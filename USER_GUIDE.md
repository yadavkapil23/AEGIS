# AEGIS: Phase 2 User & Deployment Guide

Welcome to the **AEGIS (Advanced Engine for Generative Inference & Scheduling)** User Guide. This document explains how the project works up to **Phase 2**, where to run it, how to use it, and what its real-world applications are.

---

## 1. What is AEGIS (Phase 2)?

AEGIS is an **infrastructure-first, high-performance LLM inference engine**. Unlike simple wrappers or chat applications, AEGIS is designed to be the foundational backbone of a production AI deployment. 

In its current Phase 2 state, it operates as a **hyper-optimized single-node runtime** featuring:
- **Speculative Decoding Loops**: Generates tokens rapidly by using a small "draft" model to predict text, which a large "target" model verifies in parallel.
- **Physical KV-Cache Management**: Intelligently allocates, evicts, and re-uses LLM memory directly at the C++ level (`llama.cpp` native integration).
- **Cryptographic Audit Engine**: Secures every inference request by chaining them into an immutable Merkle hash tree—making execution logs tamper-proof.
- **Security & Bulkheading**: Protects the API with rate limiters, JWT authentication, and concurrency constraints.

---

## 2. Where to Run It

Because Phase 2 relies heavily on native GPU/CPU execution via `llama.cpp` C++ bindings, you must run it on a machine capable of compiling and running heavy C++ workloads.

### Hardware Requirements
- **Local Workstation / Desktop**: A Windows/Linux machine with a dedicated NVIDIA GPU (RTX 3090, 4090, etc.) or Apple Silicon (M-series Macs).
- **Single Cloud Server**: A standalone AWS EC2 instance (e.g., `g4dn` or `p4d` instances) or RunPod/Lambda GPU instance. 

*Note: AEGIS is currently optimized to run as a highly-performant single-node runtime.*

### Software Prerequisites
- **Rust Toolchain** (1.75+)
- **LLVM & Clang** (Required for generating FFI bindings to C++ code)
- **CMake** (v3.24+)

---

## 3. How to Run It

### Environment Setup
On Windows, you must tell the compiler where your C++ tools are located before running:
```powershell
$env:PATH="C:\Program Files\CMake\bin;" + $env:PATH
$env:LIBCLANG_PATH="C:\Program Files\LLVM\bin"
```

### Running Tests & Benchmarks
To verify that the speculative decoding loops and cache evictions are working optimally on your hardware, run the benchmarking suite:
```powershell
cargo bench -p aegis-benchmarks
```

### Starting the Engine
To boot up the AEGIS API Gateway and Inference Runtime:
```powershell
cargo run --release -p aegis-gateway
```
The server will bind to port `8080` (or the port specified in your config) and begin accepting authenticated inference requests.

---

## 4. How to Use It in the Real World

Once the AEGIS server is running, you treat it like an incredibly fast, ultra-secure version of the OpenAI API.

### Flow of a Request:
1. **Your App calls AEGIS**: A user opens your front-end web app and asks a question. Your app sends an HTTP POST request to AEGIS with a JWT Bearer token.
2. **Security Gateway**: AEGIS instantly checks if your app has hit its rate limit. If traffic is too high, it queues the request.
3. **KV-Cache Scheduler**: AEGIS looks at the prompt and realizes, *"Hey, someone asked a similar question 5 minutes ago."* It instantly re-uses the memory (KV-cache blocks) from the old request instead of recalculating the context from scratch.
4. **Speculative Execution**: The engine spins up a tiny, fast model to guess the next 5 words, and feeds those guesses to the massive, smart model. If the smart model agrees, you just generated 5 words in the time it usually takes to generate 1. 
5. **Cryptographic Audit**: Before returning the response, AEGIS mathematically hashes the event into a ledger.
6. **Response**: The user gets their answer instantly.

---

## 5. Real-World Use Cases

Phase 2 AEGIS is perfect for organizations that need complete control over their inference pipeline, security, and hardware efficiency.

### 🏥 Healthcare & Financial Compliance
Because of the **Merkle Cryptographic Audit Engine**, AEGIS produces mathematically tamper-proof logs of exactly what the LLM generated and when. This is critical for HIPAA compliance or financial auditing, where you must prove to regulators that the AI gave a specific response.

### 🏢 Enterprise Code Completion
If you are running a local Copilot alternative for a 500-person engineering team, AEGIS's **KV-Cache Scheduler** shines. Since 500 developers are constantly feeding similar codebases into the model, AEGIS's ability to reuse physical cache blocks across different requests will drastically drop latency.

### 🤖 High-Speed Autonomous Agents
If you are building Agentic AI (like AutoGPT), latency is the biggest bottleneck. Because AEGIS utilizes native **Speculative Decoding**, it generates tokens fast enough for agents to chain hundreds of "thoughts" together in seconds, without needing a massive cluster of servers.
