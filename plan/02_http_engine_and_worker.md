# Task 2: HTTP Client Engine, Connection Pooling & Worker Loop

## 1. Goal & Context
Implement the high-performance HTTP client factory, tuned specifically for 700+ concurrent persistent connections without socket churning, implement OS file descriptor limit inspection, and author the core asynchronous worker task execution loop.

---

## 2. Target Files
- `src/os/mod.rs`: Cross-platform OS sanity check interface.
- `src/os/unix.rs`: Unix/macOS file descriptor limit (`rlimit` / `ulimit -n`) query and warning.
- `src/client.rs`: `HttpClientFactory` constructing `reqwest::Client` with tuned connection pooling.
- `src/engine/worker.rs`: Single worker task loop executing HTTP requests and sampling timings.
- `tests/client_worker_test.rs`: Unit and integration test for worker execution loop against wiremock.

---

## 3. Detailed Specifications

### 3.1 OS File Descriptor Check (`src/os/mod.rs`)
Under 700 concurrent connections, each connection requires a socket file descriptor. If OS limit is low (e.g. macOS default 256), the load generator will fail with `EMFILE: Too many open files`.
- Inspect current soft limit (`RLIMIT_NOFILE`).
- If current limit $< 4096$, print a visible warning message recommending the user increase limits:
  `Warning: OS file descriptor limit is 256 (recommended >= 4096). Run 'ulimit -n 65536' before testing.`
- Return structured `OsLimits` info for inclusion in startup logs.

### 3.2 HTTP Client Builder (`src/client.rs`)
Configure `reqwest::ClientBuilder` with:
```rust
pub fn build_client(config: &BenchmarkConfig) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .pool_max_idle_per_host(config.concurrency)
        .tcp_nodelay(true)
        .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
        .timeout(config.timeout)
        .redirect(reqwest::redirect::Policy::none()) // benchmark target directly
        .user_agent("rusty-loader/0.1.0")
        .build()
        .map_err(Into::into)
}
```

### 3.3 Asynchronous Worker Task (`src/engine/worker.rs`)
Each worker runs as a detached or managed Tokio task (`tokio::spawn`):
```rust
pub struct Worker {
    id: usize,
    client: reqwest::Client,
    config: Arc<BenchmarkConfig>,
    sender: tokio::sync::mpsc::UnboundedSender<MetricEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
}
```
**Execution Loop Details:**
1. Check `cancel_token.is_cancelled()`. If true, exit loop immediately.
2. Build `reqwest::Request`:
   - URL, Method, Headers, optional body.
3. Timestamp start:
   `let start = std::time::Instant::now();`
4. Execute request:
   `match client.execute(request).await`
5. Consume full response body:
   - To capture true end-to-end network latency and reuse connections, response body must be drained:
     `let body_bytes = resp.bytes().await;`
6. Calculate elapsed duration:
   `let latency = start.elapsed();`
7. Package `MetricEvent`:
   - On success: status code, latency, payload byte size, error = None.
   - On network error or timeout: classify error string (e.g. "Timeout", "ConnectError"), latency, bytes = 0.
8. Send to metrics aggregator:
   `let _ = sender.send(event);`
9. Loop without sleep until cancelled.

---

## 4. Verification & Testing

### Automated Tests
Run:
```bash
cargo check
cargo test --test client_worker_test
```

### Test Cases in `tests/client_worker_test.rs`:
1. **Connection Pooling Test:** Build client and verify default pool limits.
2. **Worker Single Iteration Test:** Run a worker task against a mock HTTP endpoint returning 200 OK with a 50-byte body; verify emitted `MetricEvent` has matching status, body size, and sensible latency ($> 0$).
3. **Worker Cancellation Test:** Verify worker terminates promptly when cancellation token triggers.
4. **Timeout Handling Test:** Wiremock delayed response exceeding client timeout; verify worker emits error event without panic.

---

## 5. Sub-Agent Prompt Template

```text
You are executing Task 2 of the rusty-loader project.
Review plan/02_http_engine_and_worker.md for instructions.
Tasks to perform:
1. Implement OS sanity check in src/os/mod.rs (querying ulimit/rlimit on Unix and logging warnings if < 4096).
2. Implement src/client.rs with tuned connection pooling (pool_max_idle_per_host, tcp_nodelay, tcp_keepalive, redirect policy).
3. Implement src/engine/worker.rs with Tokio worker task loop capturing high-resolution timings and consuming body bytes.
4. Write tests in tests/client_worker_test.rs using wiremock to test successful response, timeout handling, and graceful cancellation.
5. Verify with `cargo test --test client_worker_test`.
```

---

## 6. Acceptance Criteria Checklist
- [ ] `HttpClientFactory` applies `pool_max_idle_per_host`, `tcp_nodelay`, and `tcp_keepalive`.
- [ ] OS file descriptor check warns cleanly if limit $< 4096$.
- [ ] Worker task measures accurate duration by awaiting full body byte consumption.
- [ ] Timeouts and connection drops generate error metric events without causing panics.
- [ ] Worker loop respects cancellation token cleanly.
