# Task 5: Terminal User Experience, Live Telemetry & Reporting

## 1. Goal & Context
Implement the user-facing telemetry and reporting layer. This includes an interactive real-time progress bar powered by `indicatif`, formatted terminal summary tables rendered with `comfy-table` and `colored`, and automated JSON serialization of full test benchmarks to disk.

---

## 2. Target Files
- `src/reporter/mod.rs`: Reporter interface dispatching between live UX, terminal tables, and JSON export.
- `src/reporter/live.rs`: Live terminal progress bar using `indicatif` reading from `LiveCounters`.
- `src/reporter/table.rs`: Formatted multi-table terminal summary using `comfy-table` and ANSI colors.
- `src/reporter/json.rs`: JSON exporter writing formatted benchmark statistics to disk.
- `tests/reporter_test.rs`: Tests for table rendering format, JSON file export, and serialization accuracy.

---

## 3. Detailed Specifications

### 3.1 Live Telemetry Progress Bar (`src/reporter/live.rs`)
Using `indicatif::ProgressBar`:
- Progress bar tracks duration: `0s` to `duration`.
- Dynamic template:
  `[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s | Req: {total} | RPS: {rps:.1} | Err: {errors}`
- Ticker task refreshes every 100ms reading `LiveCounters` atomically:
  - Computes instantaneous RPS: `(requests_now - requests_prev) / delta_t`.
- On completion or Ctrl+C, progress bar finishes cleanly and is cleared to prevent line clobbering.

### 3.2 Terminal Summary Tables (`src/reporter/table.rs`)
Render 3 distinct tables with `comfy-table`:

#### Table 1: Benchmark Overview & Throughput
| Metric | Value |
|---|---|
| Target URL | `https://api.example.com/v1/ping` |
| Concurrency (VUs) | `700` |
| Total Duration | `60.00 s` |
| Total Requests | `124,500` |
| Successful (2xx) | `124,480 (99.98%)` |
| Errors (4xx/5xx/Net) | `20 (0.02%)` |
| Throughput (Avg RPS) | `2,075.0 req/s` |
| Total Data Transferred | `52.4 MB` |
| Data Transfer Rate | `894.2 KB/s` |

#### Table 2: Latency Percentiles (HDR Histogram)
| Percentile | Latency |
|---|---|
| Min | `0.45 ms` |
| Mean | `4.21 ms` |
| Median ($p50$) | `3.80 ms` |
| $p90$ | `7.10 ms` |
| $p95$ | `11.40 ms` |
| $p99$ | `18.90 ms` |
| $p99.9$ | `45.20 ms` |
| Max | `120.50 ms` |
| Std Dev | `3.15 ms` |

*Color coding:*
- Latencies $< 50 \text{ ms}$: Green.
- Latencies between $50 \text{ ms}$ and $200 \text{ ms}$: Yellow.
- Latencies $> 200 \text{ ms}$: Red.

#### Table 3: HTTP Status Code Breakdown
| Status Code | Description | Count | Percentage |
|---|---|---|---|
| `200` | `OK` | `124,480` | `99.98%` |
| `502` | `Bad Gateway` | `20` | `0.02%` |

#### Table 4 (Conditional): Error Breakdown
If transport or connection errors occurred (e.g. timeouts, DNS failures), render an error summary table detailing category and count.

### 3.3 JSON Report Exporter (`src/reporter/json.rs`)
If `--output-json <path>` is provided:
- Format `BenchmarkStats` into pretty-printed JSON.
- Include execution metadata: timestamp, URL, concurrency, duration, method, headers.
- Write to file path, ensuring parent directories exist.

---

## 4. Verification & Testing

### Automated Tests
Run:
```bash
cargo check
cargo test --test reporter_test
```

### Test Cases in `tests/reporter_test.rs`:
1. **Table Rendering:** Render tables with sample `BenchmarkStats`; assert expected strings (e.g., `p99`, `req/s`, `200 OK`) are present.
2. **JSON Output File Creation:** Pass temporary file path to JSON exporter; verify file is created, valid JSON is written, and structure matches `BenchmarkStats`.
3. **Color Formatting:** Verify colored string helper functions format properly without corrupting text content.

---

## 5. Sub-Agent Prompt Template

```text
You are executing Task 5 of the rusty-loader project.
Review plan/05_telemetry_and_reporting.md for instructions.
Tasks to perform:
1. Implement live Indicatif progress bar in src/reporter/live.rs with 100ms atomic counter polling.
2. Implement comfy-table terminal report in src/reporter/table.rs with throughput, HDR latency percentiles, and status distribution tables.
3. Implement JSON export writer in src/reporter/json.rs.
4. Wire reporting into src/reporter/mod.rs.
5. Create tests in tests/reporter_test.rs verifying table outputs and JSON file persistence.
6. Verify with `cargo test --test reporter_test`.
```

---

## 6. Acceptance Criteria Checklist
- [ ] Live progress bar accurately displays elapsed time, total requests, instantaneous RPS, and errors.
- [ ] Final summary tables are clearly formatted with `comfy-table` and color-coded.
- [ ] Latency percentiles correctly render sub-millisecond precision (e.g. `1.24 ms`).
- [ ] HTTP status breakdown lists all encountered status codes.
- [ ] JSON export generates valid, complete JSON report when requested.
- [ ] All reporter tests pass.
