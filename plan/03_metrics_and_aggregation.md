# Task 3: High-Fidelity Metrics Aggregation & Statistics

## 1. Goal & Context
Implement the high-fidelity metrics aggregation system. This entails defining `MetricEvent`, configuring an HDR Histogram (`hdrhistogram`) for recording latency values from 1 µs to 60,000 ms with 3 significant figures of precision, maintaining atomic counters for zero-contention live telemetry polling, and generating detailed summary statistics ready for table and JSON serialization.

---

## 2. Target Files
- `src/metrics/mod.rs`: Metrics collector coordinator and channel receiver task.
- `src/metrics/event.rs`: Data structures for request event notifications (`MetricEvent`).
- `src/metrics/histogram.rs`: Wrapper around `hdrhistogram::Histogram<u64>` handling microsecond conversions and percentiles.
- `src/metrics/stats.rs`: `BenchmarkStats` summary structures with Serde serialization.
- `tests/metrics_test.rs`: Unit tests for histogram recording, percentile calculations, and serialization.

---

## 3. Detailed Specifications

### 3.1 MetricEvent (`src/metrics/event.rs`)
```rust
#[derive(Debug, Clone)]
pub struct MetricEvent {
    pub latency: std::time::Duration,
    pub status_code: Option<reqwest::StatusCode>,
    pub bytes_received: usize,
    pub error: Option<String>,
}
```

### 3.2 Atomic Counters (`src/metrics/stats.rs`)
To allow the live progress bar to render smooth real-time updates without locking the histogram or blocking the event channel:
```rust
#[derive(Default)]
pub struct LiveCounters {
    pub total_requests: std::sync::atomic::AtomicU64,
    pub successful_requests: std::sync::atomic::AtomicU64, // 2xx
    pub client_errors: std::sync::atomic::AtomicU64,       // 4xx
    pub server_errors: std::sync::atomic::AtomicU64,       // 5xx
    pub other_statuses: std::sync::atomic::AtomicU64,
    pub transport_errors: std::sync::atomic::AtomicU64,    // timeouts, connect errors
    pub total_bytes: std::sync::atomic::AtomicU64,
}
```

### 3.3 HDR Histogram (`src/metrics/histogram.rs`)
Initialize histogram with bounds:
- Minimum measurable value: `1` microsecond.
- Maximum measurable value: `60_000_000` microseconds (60 seconds).
- Significant figures: `3`.

```rust
pub struct LatencyHistogram {
    histogram: hdrhistogram::Histogram<u64>,
}

impl LatencyHistogram {
    pub fn new() -> Result<Self, hdrhistogram::CreationError> {
        let histogram = hdrhistogram::Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)?;
        Ok(Self { histogram })
    }

    pub fn record(&mut self, latency: std::time::Duration) {
        let micros = latency.as_micros() as u64;
        let clamped = micros.clamp(1, 60_000_000);
        let _ = self.histogram.record(clamped);
    }

    pub fn percentile(&self, p: f64) -> std::time::Duration {
        std::time::Duration::from_micros(self.histogram.value_at_percentile(p))
    }
}
```

### 3.4 Summary Statistics Model (`src/metrics/stats.rs`)
Structured for both terminal formatting and JSON output:
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkStats {
    pub total_duration_secs: f64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub client_errors: u64,
    pub server_errors: u64,
    pub transport_errors: u64,
    pub requests_per_sec: f64,
    pub total_bytes: u64,
    pub bytes_per_sec: f64,
    pub latencies: LatencyDistribution,
    pub status_distribution: std::collections::HashMap<u16, u64>,
    pub error_breakdown: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LatencyDistribution {
    pub min_ms: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub p99_9_ms: f64,
    pub max_ms: f64,
    pub std_dev_ms: f64,
}
```

### 3.5 Collector Consumer Task (`src/metrics/mod.rs`)
- Continuously receives `MetricEvent` from `tokio::sync::mpsc::UnboundedReceiver<MetricEvent>`.
- Increments atomic counters immediately.
- Updates histogram and status maps.
- On channel closure (when workers finish and senders drop), computes the final `BenchmarkStats`.

---

## 4. Verification & Testing

### Automated Tests
Run:
```bash
cargo check
cargo test --test metrics_test
```

### Test Cases in `tests/metrics_test.rs`:
1. **Histogram Accuracy:** Record 10,000 synthetic latencies following known distribution; verify $p50, p90, p99$ match within expected tolerance.
2. **Boundary Clamping:** Verify values $< 1 \mu s$ or $> 60 s$ record safely without crashing.
3. **Status Code Counter:** Verify 200, 404, 500, and transport errors correctly increment atomic counters and status frequency map.
4. **JSON Serialization:** Serialize `BenchmarkStats` to JSON and deserialize back, asserting all fields match.

---

## 5. Sub-Agent Prompt Template

```text
You are executing Task 3 of the rusty-loader project.
Review plan/03_metrics_and_aggregation.md for instructions.
Tasks to perform:
1. Define MetricEvent in src/metrics/event.rs.
2. Implement HdrHistogram wrapper in src/metrics/histogram.rs with 1us - 60s bounds and 3 sig figs.
3. Implement LiveCounters and BenchmarkStats with Serde support in src/metrics/stats.rs.
4. Implement metrics collector receiver loop in src/metrics/mod.rs.
5. Create comprehensive tests in tests/metrics_test.rs covering histogram recording, percentiles, error classification, and JSON serialization.
6. Run `cargo test --test metrics_test` to verify.
```

---

## 6. Acceptance Criteria Checklist
- [ ] `MetricEvent` captures latency, status code, byte count, and error strings.
- [ ] `HdrHistogram` properly bounds and records microsecond latencies with 3 significant figures.
- [ ] `LiveCounters` provides lockless atomic access for live telemetry.
- [ ] All latency percentiles ($p50, p90, p95, p99, p99.9$) are computed accurately.
- [ ] `BenchmarkStats` serializes to clean JSON.
- [ ] All unit tests in `metrics_test.rs` pass.
