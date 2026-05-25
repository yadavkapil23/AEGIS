# AEGIS - LLM Gateway & Inference Orchestration System

**Advanced Engine for Generative Inference & Scheduling**

A production-ready, highly-optimized LLM gateway and systems infrastructure project built in Rust. AEGIS is designed to be the foundational backbone of an enterprise AI deployment, prioritizing deep physical memory control, speculative decoding, and cryptographic observability.

---

## 🎯 What is AEGIS?

AEGIS is not just a model wrapper; it is an **infrastructure-first, high-performance LLM inference engine**. It sits between your applications and your models, providing:

- **Native C++ LLM Integration**: Uses FFI bindings (`llama-cpp-2`) for deep, zero-overhead physical control of the model.
- **Speculative Decoding Loops**: Generates tokens rapidly by using a small "draft" model to predict text, which a large "target" model verifies in parallel.
- **Physical KV-Cache Management**: Intelligently allocates, evicts, and re-uses LLM memory directly at the C++ level.
- **Cryptographic Audit Engine**: Secures every inference request by chaining them into an immutable Merkle hash tree—making execution logs tamper-proof.
- **Security & Bulkheading**: Protects the API with rate limiters, JWT authentication, and concurrency constraints.
- **PostgreSQL Persistence**: Stores logs, API keys, and audit trails securely.
- **Real-time Observability**: Fully instrumented with OpenTelemetry, Prometheus, and Grafana.

---

## 🚀 Quick Start (Phase 2 Local Deployment)

Because AEGIS relies heavily on native GPU/CPU execution via C++ bindings, you must run it on a machine capable of compiling C++ workloads (e.g., Windows with CMake/LLVM, or Linux with build-essential).

### Prerequisites
- **Rust Toolchain** (1.75+)
- **LLVM & Clang** (Required for generating FFI bindings)
- **CMake** (v3.24+)
- **Docker & Docker Compose** (For the Database and Observability stack)

### Step 1: Start the Infrastructure Services
AEGIS requires PostgreSQL for Merkle logs and API keys, as well as Prometheus/Grafana for telemetry.
```bash
docker-compose -f docker-compose-services.yml up -d
```

### Step 2: Configure Environment (Windows Example)
Tell the Rust compiler where your C++ tools are located before compiling:
```powershell
$env:PATH="C:\Program Files\CMake\bin;" + $env:PATH
$env:LIBCLANG_PATH="C:\Program Files\LLVM\bin"
```

### Step 3: Run Benchmarks (Optional)
To verify that the speculative decoding loops and cache evictions are working optimally on your hardware:
```bash
cargo bench -p aegis-benchmarks
```

### Step 4: Start the Engine
Boot up the AEGIS API Gateway and Inference Runtime:
```bash
cargo run --release -p aegis-gateway
```
The server will bind to `0.0.0.0:8080` and begin accepting authenticated inference requests.

---

## 🏢 Real-World Use Cases

AEGIS is perfect for organizations that need complete control over their inference pipeline, security, and hardware efficiency.

### 🏥 Healthcare & Financial Compliance
Because of the **Merkle Cryptographic Audit Engine**, AEGIS produces mathematically tamper-proof logs of exactly what the LLM generated and when. This is critical for HIPAA compliance or financial auditing, where you must prove to regulators that the AI gave a specific response.

### 🏢 Enterprise Code Completion
If you are running a local Copilot alternative for an engineering team, AEGIS's **KV-Cache Scheduler** shines. Since developers constantly feed similar codebases into the model, AEGIS's ability to reuse physical cache blocks across different requests will drastically drop latency.

### 🤖 High-Speed Autonomous Agents
If you are building Agentic AI, latency is the biggest bottleneck. Because AEGIS utilizes native **Speculative Decoding**, it generates tokens fast enough for agents to chain hundreds of "thoughts" together in seconds, without needing a massive cluster of servers.

---

## 🗺️ Project Roadmap

- **[x] Phase 1**: Core Gateway, Router Logic, Authentication, and Simulated Execution.
- **[x] Phase 2**: Native C++ Integration, Speculative Decoding, KV-Cache Physical Management, and Merkle Auditing.

