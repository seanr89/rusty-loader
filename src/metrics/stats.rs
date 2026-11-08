use crate::metrics::event::MetricEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Lockless atomic counters updated in real-time by the collector or workers
/// and polled by the live terminal progress bar.
#[derive(Debug, Default)]
pub struct LiveCounters {
    pub total_requests: AtomicU64,
    pub successful_requests: AtomicU64, // 2xx
    pub client_errors: AtomicU64,       // 4xx
    pub server_errors: AtomicU64,       // 5xx
    pub other_statuses: AtomicU64,      // 1xx, 3xx, etc.
    pub transport_errors: AtomicU64,    // timeouts, connect errors
    pub total_bytes: AtomicU64,
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
                if (200..300).contains(&code) {
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

    /// Reads the sum of client (4xx), server (5xx), and transport errors.
    pub fn total_errors(&self) -> u64 {
        self.client_errors() + self.server_errors() + self.transport_errors()
    }

    /// Reads the total number of response body bytes received so far.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }
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
