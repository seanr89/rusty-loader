# [EPIC] Build High-Performance Rust API Load Testing CLI

## 1. Overview & Objectives

Build a lightweight, concurrent command-line load testing tool written in Rust capable of driving sustained workloads against HTTP/REST APIs. The tool must support a default baseline of **700 concurrent virtual users (VUs)** executing continuous query and ping workflows while recording sub-millisecond latency percentiles ($p50$, $p90$, $p95$, $p99$), throughput (requests per second), and HTTP status distributions without introducing client-side latency skew.

### Key Goals
- **High Concurrency:** Support $\ge 700$ concurrent async workers with low memory footprint ($< 50 \text{ MB}$ RSS).
- **Zero-GC Measurement Fidelity:** Leverage Rust's zero-cost async primitives to guarantee that measured latency reflects server response times rather than client runtime pauses.
- **Connection Optimization:** Ensure proper HTTP connection pooling and keep-alive reuse to prevent local ephemeral port and file descriptor exhaustion.
- **Accurate Statistics:** Use High Dynamic Range (HDR) histograms to capture latencies accurately without precision degradation or coordinated omission.

---

## 2. Technical Stack & Dependencies

```toml
[dependencies]
# Async Runtime & Networking
tokio = { version = "1.40", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "stream"] }

# CLI Parsing & UX
clap = { version = "4.5", features = ["derive", "env"] }
indicatif = { version = "0.17", features = ["tokio"] }
comfy-table = "7.1"
colored = "2.1"

# Telemetry & Metrics
hdrhistogram = "7.5"
chrono = "0.4"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Error Handling & Utilities
anyhow = "1.0"
```

---

## 3. High-Level Architecture

```
                       ┌────────────────────────┐
                       │       CLI Parser       │
                       │     (clap derive)      │
                       └───────────┬────────────┘
                                   │
                                   ▼
                       ┌────────────────────────┐
                       │     Test Controller    │
                       │ (Ramp-up, Task Spawner)│
                       └───────────┬────────────┘
                                   │
          ┌────────────────────────┼────────────────────────┐
          │                        │                        │
          ▼                        ▼                        ▼
┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│ Worker 1 (Async) │     │ Worker 2 (Async) │ ... │Worker 700 (Async)│
└─────────┬────────┘     └─────────┬────────┘     └─────────┬────────┘
          │                        │                        │
          └────────────────────────┼────────────────────────┘
                                   │ Shared MPSC Channel
                                   ▼
                       ┌────────────────────────┐
                       │   Metrics Collector    │
                       │ (HdrHistogram + Atomics│
                       └───────────┬────────────┘
                                   │
                                   ▼
                       ┌────────────────────────┐
                       │    Terminal Output     │
                       │ (Live Bar + Summary)   │
                       └────────────────────────┘
```

---

## 4. Step-by-Step Implementation Roadmap

### Phase 1: Project Setup & CLI Argument Specification
- [ ] Initialize Cargo workspace: `cargo init --bin api-load-tester`.
- [ ] Define CLI interface with `clap` derive macros:
  - `--url` (`-u`): Target endpoint (required).
  - `--concurrency` (`-c`): Number of concurrent workers (default: `700`).
  - `--duration` (`-d`): Total execution duration in seconds or duration string (e.g., `30s`, `2m`).
  - `--method` (`-m`): HTTP method (`GET`, `POST`, `PUT`, `DELETE`; default: `GET`).
  - `--header` (`-H`): Key-value request headers (repeatable).
  - `--body` (`-b`): Raw body string or path to JSON payload.
  - `--timeout` (`-t`): Request timeout in milliseconds (default: `5000ms`).
  - `--ramp-up`: Ramp-up time in seconds to incrementally reach target concurrency.
  - `--output-json`: Optional path to dump full benchmark metrics as JSON.

### Phase 2: HTTP Client Configuration & Connection Pooling
- [ ] Configure `reqwest::Client` with pooling limits tuned for high concurrency:
  - Set `pool_max_idle_per_host(700)` to prevent socket churning.
  - Set `tcp_nodelay(true)` for lowest packet transmission latency.
  - Set `tcp_keepalive(Some(Duration::from_secs(60)))`.
  - Disable automatic redirects if benchmarking exact endpoint latency.
- [ ] Implement system-level sanity checks:
  - Check OS file descriptor limits (`ulimit -n` on Linux/macOS) and warn if $< 4096$.

### Phase 3: Async Engine & Concurrency Orchestration
- [ ] Implement the `WorkerPool` using Tokio tasks:
  - Spawn $N$ worker tasks using `tokio::spawn`.
  - Pass an `Arc<reqwest::Client>` across all tasks.
  - Implement a staggered spawn loop if `--ramp-up` is specified (e.g., interval between worker spawns $= \frac{\text{ramp\_duration}}{\text{concurrency}}$).
- [ ] Implement the worker loop:
  - Each worker loops continuously until a `tokio::sync::watch` cancellation token or deadline fires.
  - In each iteration:
    1. Sample high-resolution timestamp: `let start = std::time::Instant::now()`.
    2. Dispatch HTTP request.
    3. Await response headers and consume payload bytes (to measure full round-trip transmission).
    4. Calculate elapsed duration: `let latency = start.elapsed()`.
    5. Emit result metric event to an asynchronous aggregation channel (`tokio::sync::mpsc::unbounded_channel`).

### Phase 4: High-Fidelity Metrics Aggregation
- [ ] Create `MetricEvent` data structure:
  ```rust
  pub struct MetricEvent {
      pub latency: std::time::Duration,
      pub status_code: reqwest::StatusCode,
      pub bytes_received: usize,
      pub error: Option<String>,
  }
  ```
- [ ] Implement dedicated metrics consumer task:
  - Use `hdrhistogram::Histogram<u64>` configured for dynamic ranges from 1 microsecond to 60,000 milliseconds with 3 significant figures.
  - Maintain counters for total requests, successful requests (2xx), client errors (4xx), server errors (5xx), connection timeouts, and total bytes transferred.
  - Maintain non-blocking lockless updates to prevent worker backpressure.

### Phase 5: Terminal User Experience & Live Telemetry
- [ ] Add live progress tracking using `indicatif`:
  - Show running timer, current request count, real-time RPS, and active errors.
- [ ] Implement test teardown and final summary reporter using `comfy-table`:
  - Render a clean summary table detailing:
    - **Throughput:** Total Requests, Successful Requests, Failed Requests, Avg RPS.
    - **Latency Distribution:** Min, Mean, Median ($p50$), $p90$, $p95$, $p99$, Max.
    - **Network:** Total Data Transferred, Avg Transfer Rate.
    - **HTTP Status Breakdown:** Table of status codes encountered (e.g., `200 OK`, `429 Too Many Requests`, `502 Bad Gateway`).
- [ ] If `--output-json` is provided, serialize the final histogram and summary stats to disk.

### Phase 6: Validation, Load Verification & OS Tuning
- [ ] Stand up a lightweight mock HTTP server (using `wiremock` or an Axum test endpoint).
- [ ] Benchmark the CLI tool against `127.0.0.1` at 700 concurrent connections:
  - Monitor local CPU usage and memory footprint.
  - Verify that no socket leaks (`TIME_WAIT` explosions) occur.
- [ ] Validate metric accuracy against an established load generator (e.g., `drill` or `k6`).

---

## 5. Acceptance Criteria

- [ ] **Capacity:** Successfully runs 700 concurrent async workers against a target endpoint without crashing, dropping threads, or leaking memory.
- [ ] **Resource Footprint:** Resident Set Size (RSS) stays below 50 MB during full 700-VU sustained load.
- [ ] **Accurate Percentiles:** Percentiles ($p50, p90, p95, p99$) match HDR distribution standards within $\pm 2\%$ variance of network controls.
- [ ] **Graceful Termination:** Catching `SIGINT` (`Ctrl+C`) triggers an orderly shutdown, draining pending in-flight metrics and still printing the final partial summary report.
- [ ] **Fault Tolerance:** Network timeouts and connection drops are tracked as error categories rather than causing a panic.

---

## 6. Example CLI Usage Pattern

```bash
# Basic run with 700 concurrent users for 60 seconds
api-load-tester \
  --url "https://api.example.com/v1/ping" \
  --concurrency 700 \
  --duration 60s \
  --timeout 2000

# Query test with custom headers, ramp-up, and JSON output
api-load-tester \
  --url "https://api.example.com/v1/query" \
  --method POST \
  --header "Authorization: Bearer token123" \
  --header "Content-Type: application/json" \
  --body '{"query":"active_records","limit":50}' \
  --concurrency 700 \
  --ramp-up 10 \
  --duration 120s \
  --output-json report.json
```