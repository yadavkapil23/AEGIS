# AEGIS v3.0.0 - Quickstart Guide

This guide will get the AEGIS Native Inference Engine running on your local machine.

## Prerequisites
Because AEGIS compiles native C++ bindings for the GPU/CPU, you must have:
* **Rust Toolchain** (`rustup default stable`)
* **CMake** (v3.24+)
* **LLVM & Clang** (Required for the `bindgen` crate to generate FFI code)
* **Docker** (For the Database and Telemetry)

## Step 1: Boot Infrastructure
Start the PostgreSQL database (for API keys and Merkle logs) and the Prometheus/Grafana observability stack:
```bash
docker-compose -f docker-compose-services.yml up -d
```

## Step 2: Configure Environment Variables
You must tell the Rust compiler where your C++ build tools are located.
**(Windows PowerShell Example):**
```powershell
$env:PATH="C:\Program Files\CMake\bin;" + $env:PATH
$env:LIBCLANG_PATH="C:\Program Files\LLVM\bin"
```

## Step 3: Run the Native Server
Compile and run the Actix-Web Gateway. (Note: The first run takes several minutes because it compiles the C++ `llama.cpp` source code from scratch).
```bash
cargo run --release -p aegis-gateway
```
You will see logs confirming successful connection to PostgreSQL, Database Migrations running, and the LLM Backend initializing on `0.0.0.0:8080`.

## Step 4: Test the API
You can now send high-speed, native inference requests to the engine:

```bash
curl -X POST http://localhost:8080/infer \
  -H "Content-Type: application/json" \
  -H "X-API-Key: YOUR_API_KEY_HERE" \
  -d '{
    "model": "native-llama",
    "prompt": "Write a high performance Rust function.",
    "max_tokens": 100
  }'
```

## Step 5: View Metrics
Open your browser and navigate to **http://localhost:3000** to view the Grafana dashboard tracking your GPU memory, latency, and cache hit rates in real-time.
