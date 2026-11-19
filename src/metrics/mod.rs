//! High-fidelity metrics aggregation and HDR histogram statistics.

pub mod event;
pub mod histogram;
pub mod stats;

pub use event::MetricEvent;
pub use histogram::LatencyHistogram;
pub use stats::{BenchmarkStats, LatencyDistribution, LiveCounters};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

/// Stateful aggregator that consumes [`MetricEvent`]s, updates atomic [`LiveCounters`],
/// records latencies in a [`LatencyHistogram`], and tracks HTTP status and error distributions.
pub struct MetricsCollector {
    counters: Arc<LiveCounters>,
    histogram: LatencyHistogram,
    status_distribution: HashMap<u16, u64>,
    error_breakdown: HashMap<String, u64>,
}

impl MetricsCollector {
    /// Creates a new `MetricsCollector` sharing the given `LiveCounters`.
    pub fn new(counters: Arc<LiveCounters>) -> Result<Self, hdrhistogram::CreationError> {
        Ok(Self {
            counters,
            histogram: LatencyHistogram::new()?,
            status_distribution: HashMap::new(),
            error_breakdown: HashMap::new(),
        })
    }

    /// Returns a reference to the shared [`LiveCounters`].
    pub fn counters(&self) -> &Arc<LiveCounters> {
        &self.counters
    }

    /// Returns a reference to the underlying [`LatencyHistogram`].
    pub fn histogram(&self) -> &LatencyHistogram {
        &self.histogram
    }

    /// Records a single [`MetricEvent`] into the atomic counters, HDR histogram, and breakdown maps.
    pub fn record(&mut self, event: MetricEvent) {
        self.counters.record_event(&event);
        self.histogram.record(event.latency);

        if let Some(status) = event.status_code {
            *self.status_distribution.entry(status.as_u16()).or_insert(0) += 1;
        }

        if let Some(err) = event.error {
            *self.error_breakdown.entry(err).or_insert(0) += 1;
        } else if event.status_code.is_none() {
            *self
                .error_breakdown
                .entry("UnknownTransportError".to_string())
                .or_insert(0) += 1;
        }
    }

    /// Consumes events from `receiver` until the channel closes, then computes [`BenchmarkStats`]
    /// using the elapsed time since `start_time`.
    pub async fn run(
        mut self,
        mut receiver: UnboundedReceiver<MetricEvent>,
        start_time: Instant,
    ) -> BenchmarkStats {
        while let Some(event) = receiver.recv().await {
            self.record(event);
        }
        self.finalize(start_time.elapsed())
    }

    /// Finalizes aggregation for a given total benchmark duration and returns [`BenchmarkStats`].
    pub fn finalize(self, elapsed: Duration) -> BenchmarkStats {
        let total_duration_secs = elapsed.as_secs_f64();
        let total_requests = self.counters.total_requests();
        let successful_requests = self.counters.successful_requests();
        let client_errors = self.counters.client_errors();
        let server_errors = self.counters.server_errors();
        let other_statuses = self.counters.other_statuses();
        let transport_errors = self.counters.transport_errors();
        let total_bytes = self.counters.total_bytes();

        let (requests_per_sec, bytes_per_sec) = if total_duration_secs > 0.0 {
            (
                total_requests as f64 / total_duration_secs,
                total_bytes as f64 / total_duration_secs,
            )
        } else {
            (0.0, 0.0)
        };

        BenchmarkStats {
            total_duration_secs,
            total_requests,
            successful_requests,
            client_errors,
            server_errors,
            other_statuses,
            transport_errors,
            requests_per_sec,
            total_bytes,
            bytes_per_sec,
            latencies: self.histogram.to_distribution(),
            status_distribution: self.status_distribution,
            error_breakdown: self.error_breakdown,
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new(Arc::new(LiveCounters::new())).expect("valid default MetricsCollector")
    }
}

/// Spawns a background Tokio task that collects [`MetricEvent`]s from `receiver` until all senders drop.
pub fn spawn_collector(
    receiver: UnboundedReceiver<MetricEvent>,
    counters: Arc<LiveCounters>,
    start_time: Instant,
) -> JoinHandle<BenchmarkStats> {
    let collector = MetricsCollector::new(counters).expect("failed to initialize LatencyHistogram");
    tokio::spawn(async move { collector.run(receiver, start_time).await })
}
