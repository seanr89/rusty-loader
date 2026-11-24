# rusty-loader (`api-load-tester`)

High-performance, concurrent command-line HTTP/REST API load testing tool written in Rust.

`rusty-loader` (also installed as the binary alias `api-load-tester`) is engineered to drive sustained workloads at a default baseline of **700 concurrent virtual users (VUs)** while recording sub-millisecond High Dynamic Range (HDR) latency percentiles (`p50`, `p90`, `p95`, `p99`, `p99.9`), throughput (requests/sec and transfer rate), HTTP status distributions, and categorized transport errors—all within a **`< 50 MB` Resident Set Size (RSS)** memory footprint.

---

## Key Features

- **700+ Concurrent Virtual Users:** Spawns lightweight asynchronous Tokio worker tasks sharing a connection-pooled HTTP client, sustaining 700+ concurrent connections with minimal CPU and memory overhead (`~20–30 MB` RSS).
- **Zero-GC Latency Fidelity:** Eliminates runtime garbage-collection pauses and records request round-trip latencies (`Instant::now()` through full response body drain) into a 3-significant-figure `hdrhistogram::Histogram<u64>` spanning `1 µs` to `60,000 ms`.
- **Connection Reuse & Socket Optimization:** Configures `reqwest::Client` with `pool_max_idle_per_host(concurrency)`, `tcp_nodelay(true)`, `tcp_keepalive(60s)`, and disabled automatic redirects (`Policy::none()`) to prevent ephemeral port exhaustion and `TIME_WAIT` socket churn.
- **Linear Ramp-Up Orchestration:** Supports staggered worker spawning (`--ramp-up`) at $\Delta t = \frac{\text{ramp\_up}}{\text{concurrency}}$ intervals to smoothly warm up target services.
- **Live Terminal Telemetry & Summary Tables:** Displays a flicker-free `indicatif` progress bar with real-time 10 Hz sliding-window RPS and error counts, followed by color-coded `comfy-table` reports.
- **Real-Time Embedded Web Dashboard (`--gui`):** Optionally starts a zero-CDN embedded `axum` web server streaming windowed (`250ms`) HDR latency percentiles, live RPS, active VUs, HTTP status distributions, and transport errors over Server-Sent Events (SSE) with mid-run history hydration and browser-triggered graceful stop.
- **Graceful `Ctrl+C` (`SIGINT`) Shutdown:** Intercepts `SIGINT`, stops new request dispatches, allows in-flight requests up to a 3-second grace period to drain, and renders the partial benchmark report and JSON export without losing telemetry.
- **CI/CD JSON Export:** Serializes full benchmark metadata and flattened `BenchmarkStats` to disk via `--output-json <PATH>`, automatically creating parent directories if needed.
- **Pre-Flight OS Limit Inspection:** Automatically checks `RLIMIT_NOFILE` (`ulimit -n`) on Unix/macOS/Linux before starting and warns if the open file descriptor soft limit is below `4096`.

---

## Architecture

```text
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
                                   │ Unbounded MPSC Channel
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

### Module Structure

| Module | Path | Responsibility |
| :--- | :--- | :--- |
| **Changelog** | `src/changelog.rs` | Compile-time embedded `CHANGELOG.md` (`include_str!`), release parser, and `/api/about` + `/api/changelog` models |
| **CLI** | `src/cli.rs` | `clap` derive parser, HTTP method enum, `--changelog` flag, human-friendly duration/timeout/header parsers |
| **Config** | `src/config.rs` | Validated `BenchmarkConfig` builder, URL/scheme checks, and `@file` body payload resolution |
| **Client** | `src/client.rs` | `HttpClientFactory` tuning `reqwest::Client` connection pools and TCP socket options |
| **OS** | `src/os/` | Non-blocking `libc::getrlimit(RLIMIT_NOFILE)` inspection, soft-limit auto-raise, and `ulimit -n` warnings |
| **Presets** | `src/presets.rs` | Local saved preset & recent configuration store (`presets.json` with `0600` permissions) and CLI formatter |
| **Engine** | `src/engine/` | `TestController`, staggered worker spawner, `Worker` execution loop, and `SIGINT` handler |
| **Metrics** | `src/metrics/` | `MetricEvent`, lockless `LiveCounters`, `LatencyHistogram`, `TelemetryHub`, and `MetricsCollector` |
| **GUI** | `src/gui/` | Feature-gated (`gui`) embedded `axum` monitoring dashboard, multi-run `RunManager`, session-auth REST/SSE API, and offline SPA assets |
| **Desktop** | `src/desktop/` | Feature-gated (`desktop`, macOS) native `tao`/`wry` `WKWebView` window, `muda` menus, and `rfd` file dialogs |
| **Reporter** | `src/reporter/` | `indicatif` live progress bar, `comfy-table` terminal tables, and `serde_json` file exporter |

---

## Installation & Build

### Prerequisites
- **Rust:** `1.75+` (2021 edition) and Cargo

### Build Release Binaries
```bash
cargo build --release
```
This produces two identical optimized binaries in `target/release/`:
- `./target/release/rusty-loader`
- `./target/release/api-load-tester`

### Install Locally via Cargo
```bash
cargo install --path .
```

### macOS Application Bundle (Makefile)

You can package `rusty-loader` as a signed macOS `.app` bundle (`rusty-loader.app`) with version metadata populated automatically from `Cargo.toml`, install it locally, and update an existing installation in-place via the root `Makefile`:

```bash
# Build the release binary (cargo build --release)
make build

# Build and assemble target/release/bundle/rusty-loader.app
make macos-bundle

# Install signed rusty-loader.app into /Applications (requires write access or sudo)
make macos-install

# Rebuild, stop any running instance, atomically replace, re-sign, and refresh LaunchServices
make macos-update

# Remove rusty-loader.app from APP_DIR
make macos-uninstall
```

#### Installing or Updating in User Applications (`~/Applications`)
By default `APP_DIR` is `/Applications`. Override `APP_DIR` to install or update without `sudo` in your user Applications folder:

```bash
make macos-install APP_DIR=~/Applications
make macos-update APP_DIR=~/Applications
```

> **Note:** `make macos-update` requires an existing `rusty-loader.app` in `APP_DIR` and will fail with an actionable prompt if you haven't run `make macos-install` yet.

#### Code Signing & Gatekeeper Notes
- By default, bundles are signed locally with an **ad-hoc** signature (`CODESIGN_IDENTITY="-"`).
- To sign with an Apple Developer certificate instead, override `CODESIGN_IDENTITY`:
  ```bash
  make macos-install CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
  ```
- When launching a locally built, ad-hoc signed `.app` from Finder or Spotlight for the first time, macOS Gatekeeper may block or warn about an unidentified developer. You can allow it under **System Settings → Privacy & Security → Open Anyway** (or right-click / Control-click the app in Finder and choose **Open**).
- Launching `rusty-loader.app` from Finder, Spotlight, or Dock opens the **native macOS desktop UI** directly (`rusty-loader --desktop`) with interactive benchmark configuration, saved presets, live telemetry charting, run comparison, and native file open/save dialogs. Invoking the bundle entrypoint (`Contents/MacOS/rusty-loader-launcher`) from a shell with CLI flags forwards all arguments transparently to the binary.

---

## CLI Options Reference

```text
Usage: rusty-loader [OPTIONS] --url <URL>
```

| Short | Long | Default | Description |
| :--- | :--- | :--- | :--- |
| `-u` | `--url <URL>` | *(Required)* | Target HTTP/HTTPS endpoint URL (e.g., `https://api.example.com/v1/ping`) |
| `-c` | `--concurrency <N>` | `700` | Number of concurrent virtual users / async workers |
| `-d` | `--duration <DURATION>` | `30s` | Total benchmark duration (`500ms`, `30s`, `2m`, `1h`, or raw seconds `60`) |
| `-m` | `--method <METHOD>` | `GET` | HTTP request method (`GET`, `POST`, `PUT`, `DELETE`, `PATCH`, `HEAD`; case-insensitive) |
| `-H` | `--header <KEY: VALUE>` | `[]` | Custom HTTP header; repeatable (e.g., `-H "Authorization: Bearer token"`) |
| `-b` | `--body <BODY\|@FILE>` | `None` | Raw request body string or file path prefixed with `@` (e.g., `@payload.json`) |
| `-t` | `--timeout <TIMEOUT>` | `5000ms` | Per-request timeout (`100ms`, `5s`, or raw milliseconds `2000`) |
| | `--ramp-up <DURATION>` | `0s` | Ramp-up time to linearly spawn workers up to `--concurrency` (`10s`, `1m`, or raw seconds) |
| | `--output-json <PATH>` | `None` | Optional file path to export the full JSON benchmark report |
| | `--desktop` (`--ui`) | `false` | Launch the interactive macOS desktop UI window without requiring `--url` |
| | `--ui-serve` | `false` | Start the interactive configuration & telemetry web workbench on loopback |
| | `--gui` | `false` | Enable the embedded real-time web monitoring dashboard |
| | `--gui-port <PORT>` | `8080` | Port to bind the embedded GUI server (`0` assigns an ephemeral OS port) |
| | `--gui-bind <ADDR>` | `127.0.0.1` | IP address to bind the embedded GUI server (loopback-only by default) |
| | `--gui-open` | `false` | Automatically open the GUI dashboard URL in the default browser on startup |
| | `--gui-keep-alive` | `false` | Keep the GUI dashboard server running after completion until `Ctrl+C` |
| | `--changelog` | `false` | Print the embedded `CHANGELOG.md` release notes and exit |
| `-h` | `--help` | | Print help information |
| `-V` | `--version` | | Print version information |

---

## Usage Recipes

### 1. Basic 700-VU Ping Test
Run the default 700 concurrent virtual users against a health/ping endpoint for 30 seconds with a 2-second request timeout:

```bash
rusty-loader \
  --url "https://api.example.com/v1/ping" \
  --concurrency 700 \
  --duration 30s \
  --timeout 2000ms
```

### 2. POST Workload with Inline JSON & Custom Authorization Headers
Benchmark an authenticated query endpoint using `POST`, custom headers, and an inline JSON payload:

```bash
api-load-tester \
  --url "https://api.example.com/v1/query" \
  --method POST \
  --header "Authorization: Bearer token123" \
  --header "Content-Type: application/json" \
  --body '{"query":"active_records","limit":50}' \
  --concurrency 700 \
  --duration 60s
```

### 3. POST Workload Loading Payload from a File (`@path`)
Prefix the `--body` value with `@` to load the request body from disk once at startup:

```bash
rusty-loader \
  -u "https://api.example.com/v1/orders" \
  -m POST \
  -H "Content-Type: application/json" \
  -b @fixtures/order_payload.json \
  -c 500 \
  -d 45s
```

### 4. Staggered Ramp-Up Workload
Gradually scale from 0 to 700 concurrent virtual users over the first 10 seconds of a 2-minute test:

```bash
rusty-loader \
  --url "https://api.example.com/v1/search" \
  --concurrency 700 \
  --ramp-up 10s \
  --duration 2m
```

### 5. Automated CI/CD Benchmark with JSON Report Export
Export deterministic machine-readable metrics to `artifacts/report.json` for automated regression gates:

```bash
rusty-loader \
  --url "http://127.0.0.1:8080/v1/health" \
  --concurrency 700 \
  --duration 15s \
  --output-json artifacts/report.json
```

---

## macOS Desktop UI & Interactive Workbench (`--desktop` / `--ui-serve`)

Launch the interactive desktop application directly from `rusty-loader.app` or from the terminal without specifying `--url` upfront:

```bash
# Open the native macOS desktop window (WKWebView + native menus & file dialogs)
rusty-loader --desktop

# Pre-populate the desktop form from CLI flags
rusty-loader --desktop --url "https://api.example.com/v1/ping" --concurrency 700 --duration 30s

# Launch the interactive configuration workbench in your default browser (macOS or Linux)
rusty-loader --ui-serve --gui-open
```

- **Configuration Builder & Presets:** Configure target URL, HTTP method, concurrency, duration, ramp-up, timeout, repeatable headers (with masked sensitive values), and inline/file payloads via a native macOS file picker. Save and select reusable presets or recent runs stored with `0600` permissions at `~/Library/Application Support/rusty-loader/presets.json` (overridable via `RUSTY_LOADER_CONFIG_DIR`).
- **Multi-Run Telemetry & Comparison:** Start and gracefully stop sequential runs (`Cmd+Enter` / `Cmd+.`) while streaming real-time KPIs, Canvas 2D charts (throughput/VUs, windowed HDR latency percentiles, status/error classes), cumulative HDR summaries, native JSON report saving, and two-run delta comparisons.
- **Security:** Interactive endpoints require a per-session random token (`X-RL-Session` header / `HttpOnly` cookie), bind to loopback by default, and redact sensitive headers and URL credentials in run history and exports.
- **Build Feature Flags:**
  - Default (`gui` + `desktop` on macOS): `cargo build --release`
  - Web-only GUI without native desktop host: `cargo build --release --no-default-features --features gui`
  - Headless CLI-only binary: `cargo build --release --no-default-features`

---

## Real-Time GUI Monitoring Dashboard (`--gui`)

Enable the embedded web dashboard with `--gui` to monitor live throughput, active virtual users, windowed HDR latency percentiles (`p50`, `p90`, `p95`, `p99`, `max`), HTTP status distributions, and categorized transport errors from any modern browser—without external runtime or CDN dependencies.

### Local Launch & Keep-Alive Inspection
```bash
rusty-loader \
  --url "https://api.example.com/v1/ping" \
  --concurrency 700 \
  --ramp-up 10s \
  --duration 60s \
  --gui \
  --gui-open \
  --gui-keep-alive
```
- `--gui-port 0` lets the OS pick an unused port and prints the active URL on startup.
- `--gui-keep-alive` keeps the dashboard available after the test finishes (until `Ctrl+C`) so you can inspect final charts and download the JSON report.
- Sensitive request headers (`Authorization`, `Cookie`, `X-Api-Key`, tokens/secrets) and URL credentials are automatically redacted (`[REDACTED]`) in GUI responses.
- Build without the embedded GUI/desktop features at any time using `cargo build --release --no-default-features`.

### Remote Load Generators via SSH Port-Forwarding
For headless Linux load generators, keep `--gui-bind 127.0.0.1` (default) and tunnel the dashboard over SSH:

```bash
# 1. Forward local port 8080 to the remote generator's loopback dashboard port
ssh -L 8080:127.0.0.1:8080 user@remote-load-generator

# 2. Run rusty-loader --gui on the remote host, then open http://127.0.0.1:8080 locally
rusty-loader --url "https://api.example.com/v1/ping" --concurrency 700 --duration 60s --gui
```

### Embedded GUI HTTP & SSE Endpoints

| Endpoint | Method | Description |
| :--- | :--- | :--- |
| `/` | `GET` | Self-contained HTML/CSS/Canvas 2D dashboard SPA (works offline) |
| `/api/meta` | `GET` | Sanitized run configuration (`url`, `method`, `concurrency`, `phase`, redacted `headers`) |
| `/api/about` | `GET` | Application name, current version (`CARGO_PKG_VERSION`), and latest versioned release entry |
| `/api/changelog` | `GET` | Structured release list and raw embedded `CHANGELOG.md` markdown |
| `/api/history` | `GET` | Chronological ring buffer of historical `TelemetrySnapshot` ticks (`t = 0` hydration) |
| `/api/stream` | `GET` | Server-Sent Events (`snapshot` every `250ms` + final `complete` event with `BenchmarkStats`) |
| `/api/stop` | `POST` | Triggers graceful `CancellationToken` drain and partial summary generation |
| `/api/export/json` | `GET` | Downloads `rusty-loader-report.json` with redacted headers |

#### Sample JSON Report Schema
```json
{
  "timestamp": "2026-11-08T12:00:00.000000+00:00",
  "url": "http://127.0.0.1:8080/v1/health",
  "method": "GET",
  "concurrency": 700,
  "configured_duration_secs": 15.0,
  "headers": {},
  "total_duration_secs": 15.004,
  "total_requests": 245000,
  "successful_requests": 245000,
  "client_errors": 0,
  "server_errors": 0,
  "other_statuses": 0,
  "transport_errors": 0,
  "requests_per_sec": 16328.9,
  "total_bytes": 3675000,
  "bytes_per_sec": 244934.6,
  "latencies": {
    "min_ms": 0.31,
    "mean_ms": 42.1,
    "p50_ms": 41.2,
    "p90_ms": 48.5,
    "p95_ms": 51.0,
    "p99_ms": 58.4,
    "p99_9_ms": 69.1,
    "max_ms": 84.2,
    "std_dev_ms": 4.85
  },
  "status_distribution": {
    "200": 245000
  },
  "error_breakdown": {}
}
```

---

## OS Tuning for High Concurrency (`700+ VUs`)

When running 700+ concurrent connections on macOS or Linux, ensure your shell's open file descriptor soft limit is at least `4096` (recommended `65536`):

```bash
# Check current soft limit
ulimit -n

# Raise file descriptor limit for the current session
ulimit -n 65536
```

If `ulimit -n` is below `4096`, `rusty-loader` emits a non-fatal warning to `stderr` prior to starting the benchmark:
```text
Warning: OS file descriptor limit is 256 (recommended >= 4096). Run 'ulimit -n 65536' before testing.
```

---

## Verification & Testing

### Run Unit & Integration Tests
The test suite includes unit tests across all modules plus `wiremock`-powered end-to-end integration tests (`tests/integration_test.rs`) covering happy-path load testing, mixed HTTP status codes (`200`/`429`/`500`), client timeout classification, and end-to-end JSON report generation:

```bash
cargo test
```

### Run 700-VU Load & Memory Footprint Verification
Use the included verification script to build the release binary, launch a local async HTTP/1.1 keep-alive responder, drive 700 concurrent VUs for 10 seconds, sample Resident Set Size (`RSS`) every 500ms, and assert that peak memory stays strictly below `50 MB` (`51,200 KB`):

```bash
bash scripts/verify_700_vu_load.sh
```

#### Reference 700-VU Verification Results (`10s` Sustained Load)

| Metric | Measured Result | Acceptance Target |
| :--- | :--- | :--- |
| **Concurrent Virtual Users (VUs)** | `700` | $\ge 700$ |
| **Total Requests Completed (`10s`)** | `751,032` | — |
| **Successful Responses (`200 OK`)** | `751,032` (`100.00%`) | `100%` |
| **Transport / Socket Errors** | `0` (`0.00%`) | `0` |
| **Average Throughput** | `75,072.1 req/s` | — |
| **Latency `p50` / `p90` / `p99`** | `9.01 ms` / `10.34 ms` / `12.94 ms` | HDR 3 sig-figs |
| **Peak Resident Set Size (`RSS`)** | `48,960 KB` (`47.81 MB`) | $< 51,200\text{ KB}$ ($50.00\text{ MB}$) |

