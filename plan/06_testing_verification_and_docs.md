# Task 6: Mock Server Integration Testing, Verification & Docs

## 1. Goal & Context
Build the end-to-end integration test suite using `wiremock`, implement high-concurrency verification (validating 700 VUs with RSS $< 50\text{ MB}$), verify graceful termination and error handling, and update documentation (`README.md`) with usage recipes.

---

## 2. Target Files
- `tests/integration_test.rs`: Comprehensive end-to-end tests exercising the compiled binary and library.
- `scripts/verify_700_vu_load.sh`: Verification script to launch local mock server and measure memory/RSS under 700 concurrent VUs.
- `README.md`: Complete user guide, CLI argument documentation, usage recipes, and benchmark results.

---

## 3. Detailed Specifications

### 3.1 End-to-End Integration Tests (`tests/integration_test.rs`)
Implement the following integration tests using `wiremock::MockServer`:

1. **Happy Path Load Test:**
   - Stand up mock server responding with `200 OK` and sample JSON body.
   - Run benchmark with 50 concurrency for 2 seconds.
   - Assert:
     - Total requests $> 100$.
     - `successful_requests == total_requests`.
     - Latency percentiles are within valid bounds ($> 0$, min $\le$ p50 $\le$ p99 $\le$ max).

2. **Mixed HTTP Status Codes:**
   - Mock endpoint returning 70% `200 OK`, 20% `429 Too Many Requests`, 10% `500 Internal Server Error`.
   - Run benchmark and assert `status_distribution` reflects all 3 status codes.

3. **Client Timeout Handling:**
   - Configure mock endpoint with 500ms delay.
   - Run benchmark with `--timeout 100ms`.
   - Assert all requests are logged under `transport_errors` and the process exits with status 0 without panicking.

4. **JSON Output Verification:**
   - Run benchmark specifying `--output-json <temp_path>`.
   - Read temp file, parse with `serde_json::Value`, and verify all expected keys exist (`total_requests`, `latencies`, `status_distribution`).

### 3.2 700 Virtual Users Verification (`scripts/verify_700_vu_load.sh`)
Verification script that:
1. Spawns an in-process or local mock HTTP server.
2. Launches `cargo run --release -- --url http://127.0.0.1:PORT/ping -c 700 -d 10s`.
3. Samples the process RSS using `ps -o rss= -p $PID` every 500ms.
4. Asserts that peak RSS stays strictly below $50\text{ MB}$ ($51,200\text{ KB}$).
5. Checks network socket states (`netstat` / `lsof`) to ensure no runaway socket leakage.

### 3.3 Documentation Update (`README.md`)
- Detailed installation instructions (`cargo install --path .` or `cargo build --release`).
- Comprehensive argument options table matching clap derivation.
- Copy-paste recipes:
  - Basic ping test (`-u http://... -c 700 -d 30s`).
  - POST benchmark with JSON payload and custom Authorization headers.
  - Ramp-up workload (`--ramp-up 10s`).
  - Automated CI benchmark with `--output-json report.json`.
- Performance characteristics: RSS $< 50\text{ MB}$, zero-GC latency fidelity, HDR histogram percentiles.

---

## 4. Verification & Testing

### Automated Tests
Run:
```bash
cargo check
cargo test --test integration_test
bash scripts/verify_700_vu_load.sh
```

---

## 5. Sub-Agent Prompt Template

```text
You are executing Task 6 of the rusty-loader project.
Review plan/06_testing_verification_and_docs.md for instructions.
Tasks to perform:
1. Implement tests/integration_test.rs using wiremock covering happy path, mixed status codes, client timeouts, and JSON file generation.
2. Create scripts/verify_700_vu_load.sh to benchmark 700 concurrent connections against a local endpoint and assert RSS memory < 50MB.
3. Update README.md with comprehensive installation steps, CLI argument reference, recipes, and architecture description.
4. Run `cargo test` and verify all tests in the workspace pass.
5. Execute the 700 VU verification script to validate memory and stability criteria.
```

---

## 6. Acceptance Criteria Checklist
- [ ] Integration tests exercise happy path, error paths, and timeouts.
- [ ] 700 concurrent worker benchmark runs smoothly without socket or memory leaks.
- [ ] Peak RSS memory remains $< 50\text{ MB}$ under sustained 700 VU load.
- [ ] JSON report export verified end-to-end.
- [ ] `README.md` is comprehensive, accurate, and up to date.
