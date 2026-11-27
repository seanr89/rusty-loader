use crate::metrics::event::MetricEvent;
use crate::scenario::StepFailurePolicy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Lockless atomic counters updated in real-time by the collector or workers
/// and polled by the live terminal progress bar.
#[derive(Debug, Default)]
pub struct LiveCounters {
    pub total_requests: AtomicU64,
    pub successful_requests: AtomicU64, // 2xx (and passed assertions/extractors)
    pub client_errors: AtomicU64,       // 4xx
    pub server_errors: AtomicU64,       // 5xx
    pub other_statuses: AtomicU64,      // 1xx, 3xx, etc.
    pub transport_errors: AtomicU64,    // timeouts, connect errors
    pub check_failures: AtomicU64,      // sequence assertion / extraction failures on HTTP response
    pub total_bytes: AtomicU64,
    pub active_workers: AtomicUsize,
}

impl LiveCounters {
    /// Creates a new zero-initialized set of `LiveCounters`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increments the appropriate atomic counters for a single [`MetricEvent`].
    pub fn record_event(&self, event: &MetricEvent) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_bytes
            .fetch_add(event.bytes_received as u64, Ordering::Relaxed);

        match event.status_code {
            Some(status) => {
                let code = status.as_u16();
                if event.failure_kind.is_some() {
                    if (400..500).contains(&code) {
                        self.client_errors.fetch_add(1, Ordering::Relaxed);
                    } else if (500..600).contains(&code) {
                        self.server_errors.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.check_failures.fetch_add(1, Ordering::Relaxed);
                    }
                } else if (200..300).contains(&code) {
                    self.successful_requests.fetch_add(1, Ordering::Relaxed);
                } else if (400..500).contains(&code) {
                    self.client_errors.fetch_add(1, Ordering::Relaxed);
                } else if (500..600).contains(&code) {
                    self.server_errors.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.other_statuses.fetch_add(1, Ordering::Relaxed);
                }
            }
            None => {
                self.transport_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Reads the total number of requests recorded so far.
    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    /// Reads the total number of 2xx responses recorded so far.
    pub fn successful_requests(&self) -> u64 {
        self.successful_requests.load(Ordering::Relaxed)
    }

    /// Reads the total number of 4xx responses recorded so far.
    pub fn client_errors(&self) -> u64 {
        self.client_errors.load(Ordering::Relaxed)
    }

    /// Reads the total number of 5xx responses recorded so far.
    pub fn server_errors(&self) -> u64 {
        self.server_errors.load(Ordering::Relaxed)
    }

    /// Reads the total number of non-2xx/4xx/5xx HTTP responses recorded so far.
    pub fn other_statuses(&self) -> u64 {
        self.other_statuses.load(Ordering::Relaxed)
    }

    /// Reads the total number of network/transport errors recorded so far.
    pub fn transport_errors(&self) -> u64 {
        self.transport_errors.load(Ordering::Relaxed)
    }

    /// Reads the total number of non-4xx/5xx assertion/extraction failures recorded so far.
    pub fn check_failures(&self) -> u64 {
        self.check_failures.load(Ordering::Relaxed)
    }

    /// Reads the sum of client (4xx), server (5xx), transport errors, and check failures.
    pub fn total_errors(&self) -> u64 {
        self.client_errors() + self.server_errors() + self.transport_errors() + self.check_failures()
    }

    /// Reads the total number of response body bytes received so far.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }

    /// Reads the current number of active worker tasks.
    pub fn active_workers(&self) -> usize {
        self.active_workers.load(Ordering::Relaxed)
    }

    /// Increments the active worker count and returns the updated value.
    pub fn increment_active_workers(&self) -> usize {
        self.active_workers.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Decrements the active worker count and returns the updated value.
    pub fn decrement_active_workers(&self) -> usize {
        self.active_workers
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1)
    }

    /// Creates an RAII guard that increments `active_workers` now and decrements it when dropped.
    pub fn track_worker(self: &Arc<Self>) -> ActiveWorkerGuard {
        ActiveWorkerGuard::new(Arc::clone(self))
    }
}

/// RAII guard that increments `active_workers` on creation and decrements it on drop.
#[derive(Debug)]
pub struct ActiveWorkerGuard {
    counters: Arc<LiveCounters>,
}

impl ActiveWorkerGuard {
    /// Creates a new [`ActiveWorkerGuard`] and increments the active worker counter.
    pub fn new(counters: Arc<LiveCounters>) -> Self {
        counters.increment_active_workers();
        Self { counters }
    }
}

impl Drop for ActiveWorkerGuard {
    fn drop(&mut self) {
        self.counters.decrement_active_workers();
    }
}

/// Per-step aggregate metrics for an ordered multi-step API sequence run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepStats {
    pub step_index: usize,
    pub order_number: usize,
    pub step_id: String,
    pub step_name: String,
    pub method: String,
    pub target_preview: String,
    pub on_failure: StepFailurePolicy,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub client_errors: u64,
    pub server_errors: u64,
    pub other_statuses: u64,
    pub transport_errors: u64,
    pub assertion_failures: u64,
    pub extraction_failures: u64,
    pub aborted_iterations: u64,
    pub requests_per_sec: f64,
    pub latencies: LatencyDistribution,
    pub status_distribution: HashMap<u16, u64>,
    pub error_breakdown: HashMap<String, u64>,
}

/// End-to-end iteration / journey completion statistics for ordered sequence runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JourneyStats {
    pub started_iterations: u64,
    pub completed_iterations: u64,
    pub aborted_iterations: u64,
    pub completion_rate_pct: f64,
    pub iterations_per_sec: f64,
    pub aborted_by_step: HashMap<usize, u64>,
    pub duration: LatencyDistribution,
}

/// Complete per-step and end-to-end journey statistics attached to [`BenchmarkStats`] for sequences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioStats {
    pub steps: Vec<StepStats>,
    pub journeys: JourneyStats,
}

/// Complete summary statistics for a benchmark run, suitable for terminal rendering and JSON export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkStats {
    pub total_duration_secs: f64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub client_errors: u64,
    pub server_errors: u64,
    #[serde(default)]
    pub other_statuses: u64,
    pub transport_errors: u64,
    pub requests_per_sec: f64,
    pub total_bytes: u64,
    pub bytes_per_sec: f64,
    pub latencies: LatencyDistribution,
    pub status_distribution: HashMap<u16, u64>,
    pub error_breakdown: HashMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_stats: Option<ScenarioStats>,
}

/// Summary of latency percentiles and dispersion metrics in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
