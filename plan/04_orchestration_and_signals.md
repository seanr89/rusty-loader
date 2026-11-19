# Task 4: Concurrency Orchestration, Ramp-Up & Signal Handling

## 1. Goal & Context
Implement the `TestController` and concurrency orchestrator that manages spawning up to 700+ async worker tasks, coordinates staggered ramp-up schedules, manages the benchmark duration lifecycle, and captures `SIGINT` (`Ctrl+C`) for graceful shutdown with partial report generation.

---

## 2. Target Files
- `src/engine/mod.rs`: `TestController` coordinating the benchmark execution lifecycle.
- `src/engine/signals.rs`: Signal handling and cancellation listener (`Ctrl+C`).
- `src/engine/spawner.rs`: Worker task spawner with optional staggered ramp-up delays.
- `tests/orchestrator_test.rs`: Tests verifying ramp-up timing, task lifecycle, and graceful cancellation.

---

## 3. Detailed Specifications

### 3.1 Concurrency Orchestrator (`src/engine/mod.rs`)
The `TestController` coordinates the complete lifecycle:
```rust
pub struct TestController {
    config: Arc<BenchmarkConfig>,
    client: reqwest::Client,
}

impl TestController {
    pub async fn run(&self) -> Result<BenchmarkStats> { ... }
}
```

**Lifecycle Steps:**
1. **Initialize Channels & Signals:**
   - Create unbounded MPSC channel for `MetricEvent`.
   - Create `CancellationToken` for worker lifecycle.
   - Initialize `LiveCounters`.
2. **Spawn Metrics Collector Task:**
   - Spawn collector task consuming `MetricEvent`s.
3. **Spawn Worker Pool with Ramp-Up:**
   - If `ramp_up > Duration::ZERO`:
     - `stagger_interval = ramp_up / concurrency`.
     - In a loop, spawn worker task $i \in [0, \text{concurrency})$, sleeping `stagger_interval` between spawns.
   - If `ramp_up == Duration::ZERO`:
     - Spawn all `concurrency` workers immediately using `tokio::spawn`.
4. **Duration & Signal Race:**
   - Use `tokio::select!` between:
     - `tokio::time::sleep(config.duration)` (normal benchmark completion).
     - `tokio::signal::ctrl_c()` (early manual termination).
     - cancellation token already cancelled.
5. **Graceful Teardown:**
   - On completion or Ctrl+C, fire `cancellation_token.cancel()`.
   - Await all worker `JoinHandle`s (with a 3-second grace timeout).
   - Explicitly drop the controller's channel sender to signal EOF to the collector task.
   - Await the collector task `JoinHandle` to obtain the final `BenchmarkStats`.

### 3.2 Signal Handling (`src/engine/signals.rs`)
```rust
pub async fn wait_for_shutdown_signal(token: tokio_util::sync::CancellationToken) {
    if let Ok(()) = tokio::signal::ctrl_c().await {
        eprintln!("\nReceived Ctrl+C, initiating graceful shutdown...");
        token.cancel();
    }
}
```

---

## 4. Verification & Testing

### Automated Tests
Run:
```bash
cargo check
cargo test --test orchestrator_test
```

### Test Cases in `tests/orchestrator_test.rs`:
1. **Immediate Execution Test:** Run 50 workers for 1 second without ramp-up against wiremock; verify all workers start immediately and shutdown cleanly.
2. **Ramp-Up Timing Test:** Run 10 workers with a 500ms ramp-up; assert that worker spawn timestamps are staggered approximately 50ms apart.
3. **Graceful Cancellation Test:** Fire cancellation token prematurely and verify that all workers terminate without hanging and valid partial statistics are returned.

---

## 5. Sub-Agent Prompt Template

```text
You are executing Task 4 of the rusty-loader project.
Review plan/04_orchestration_and_signals.md for instructions.
Tasks to perform:
1. Implement cancellation signal listening in src/engine/signals.rs.
2. Implement staggered worker spawner in src/engine/spawner.rs supporting both instant and ramp-up spawns.
3. Implement the TestController lifecycle in src/engine/mod.rs coordinating workers, metrics collector, duration timer, and teardown.
4. Write tests in tests/orchestrator_test.rs using wiremock to test full lifecycle, ramp-up staging, and early shutdown.
5. Verify with `cargo test --test orchestrator_test`.
```

---

## 6. Acceptance Criteria Checklist
- [ ] Controller supports spawning arbitrary concurrency levels (including 700+ workers).
- [ ] Ramp-up smoothly staggers worker launch times when configured.
- [ ] Ctrl+C (`SIGINT`) triggers orderly worker shutdown without crashing or losing buffered metrics.
- [ ] EOF is properly sent to the metrics channel on exit, allowing clean collector termination.
- [ ] All orchestrator tests pass.
