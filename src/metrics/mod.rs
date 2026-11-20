//! High-fidelity metrics aggregation and HDR histogram statistics.

pub mod event;
pub mod histogram;
pub mod snapshot;
pub mod stats;

pub use event::MetricEvent;
pub use histogram::LatencyHistogram;
pub use snapshot::{
    current_timestamp_ms, TelemetryHub, TelemetryPhase, TelemetrySnapshot, WindowLatencyMs,
};
pub use stats::{ActiveWorkerGuard, BenchmarkStats, LatencyDistribution, LiveCounters};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

/// Maximum number of queued metric events drained per `recv` batch before yielding to `select!`.
const EVENT_DRAIN_BATCH_LIMIT: usize = 1024;

/// Stateful aggregator that consumes [`MetricEvent`]s, updates atomic [`LiveCounters`],
/// records latencies in a [`LatencyHistogram`], and tracks HTTP status and error distributions.
pub struct MetricsCollector {
    counters: Arc<LiveCounters>,
    histogram: LatencyHistogram,
    window_histogram: Option<LatencyHistogram>,
    telemetry_hub: Option<Arc<TelemetryHub>>,
    status_distribution: HashMap<u16, u64>,
    error_breakdown: HashMap<String, u64>,
    prev_tick_requests: u64,
    prev_tick_bytes: u64,
    last_tick_time: Option<Instant>,
}

impl MetricsCollector {
    /// Creates a new `MetricsCollector` sharing the given `LiveCounters`.
    pub fn new(counters: Arc<LiveCounters>) -> Result<Self, hdrhistogram::CreationError> {
        Ok(Self {
            counters,
            histogram: LatencyHistogram::new()?,
            window_histogram: None,
            telemetry_hub: None,
            status_distribution: HashMap::new(),
            error_breakdown: HashMap::new(),
            prev_tick_requests: 0,
            prev_tick_bytes: 0,
            last_tick_time: None,
        })
    }

    /// Attaches a [`TelemetryHub`] and initializes the windowed [`LatencyHistogram`]
    /// for periodic real-time snapshot emission.
    pub fn with_telemetry_hub(
        mut self,
        hub: Arc<TelemetryHub>,
    ) -> Result<Self, hdrhistogram::CreationError> {
        self.window_histogram = Some(LatencyHistogram::new()?);
        self.telemetry_hub = Some(hub);
        Ok(self)
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
        if let Some(ref mut win_hist) = self.window_histogram {
            win_hist.record(event.latency);
        }

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

    /// Captures a windowed [`TelemetrySnapshot`] and resets the windowed histogram in-place.
    pub fn take_window_snapshot(
        &mut self,
        start_time: Instant,
        phase_override: Option<TelemetryPhase>,
    ) -> TelemetrySnapshot {
        let now = Instant::now();
        let elapsed = now.duration_since(start_time);
        let elapsed_secs = elapsed.as_secs_f64();
        let delta_t = self
            .last_tick_time
            .map(|prev| now.duration_since(prev).as_secs_f64())
            .unwrap_or(elapsed_secs);
        self.last_tick_time = Some(now);

        let total_requests = self.counters.total_requests();
        let total_bytes = self.counters.total_bytes();
        let delta_requests = total_requests.saturating_sub(self.prev_tick_requests);
        let delta_bytes = total_bytes.saturating_sub(self.prev_tick_bytes);
        self.prev_tick_requests = total_requests;
        self.prev_tick_bytes = total_bytes;

        let window_rps = if delta_t > 0.0 {
            delta_requests as f64 / delta_t
        } else {
            0.0
        };
        let avg_rps = if elapsed_secs > 0.0 {
            total_requests as f64 / elapsed_secs
        } else {
            0.0
        };
        let bytes_per_sec = if delta_t > 0.0 {
            delta_bytes as f64 / delta_t
        } else {
            0.0
        };

        let window_latency_ms = self
            .window_histogram
            .as_mut()
            .map(|win_hist| {
                let dist = WindowLatencyMs::from(win_hist.to_distribution());
                win_hist.reset();
                dist
            })
            .unwrap_or_default();

        let (duration_secs, phase) = match &self.telemetry_hub {
            Some(hub) => (
                hub.configured_duration().as_secs_f64(),
                phase_override.unwrap_or_else(|| hub.current_phase(elapsed)),
            ),
            None => (0.0, phase_override.unwrap_or(TelemetryPhase::Running)),
        };

        TelemetrySnapshot {
            timestamp_ms: current_timestamp_ms(),
            elapsed_secs,
            duration_secs,
            phase,
            active_workers: self.counters.active_workers(),
            total_requests,
            window_rps,
            avg_rps,
            bytes_per_sec,
            total_bytes,
            successful_requests: self.counters.successful_requests(),
            client_errors: self.counters.client_errors(),
            server_errors: self.counters.server_errors(),
            other_statuses: self.counters.other_statuses(),
            transport_errors: self.counters.transport_errors(),
            window_latency_ms,
            status_distribution: self.status_distribution.clone(),
            error_breakdown: self.error_breakdown.clone(),
        }
    }

    /// Consumes events from `receiver` until the channel closes, then computes [`BenchmarkStats`]
    /// using the elapsed time since `start_time`.
    pub async fn run(
        mut self,
        mut receiver: UnboundedReceiver<MetricEvent>,
        start_time: Instant,
    ) -> BenchmarkStats {
        let Some(hub) = self.telemetry_hub.clone() else {
            while let Some(event) = receiver.recv().await {
                self.record(event);
            }
            return self.finalize(start_time.elapsed());
        };

        self.last_tick_time = Some(start_time);
        let mut ticker = tokio::time::interval(hub.tick_interval());
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Consume the immediate first tick so snapshots begin after one interval.
        ticker.tick().await;

        loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    let snapshot = self.take_window_snapshot(start_time, None);
                    hub.publish_snapshot(snapshot);
                }
                maybe_event = receiver.recv() => {
                    match maybe_event {
                        Some(event) => {
                            self.record(event);
                            for _ in 0..EVENT_DRAIN_BATCH_LIMIT {
                                match receiver.try_recv() {
                                    Ok(next) => self.record(next),
                                    Err(_) => break,
                                }
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        while let Ok(event) = receiver.try_recv() {
            self.record(event);
        }

        if self.counters.total_requests() > self.prev_tick_requests || hub.history().is_empty() {
            let final_snapshot =
                self.take_window_snapshot(start_time, Some(TelemetryPhase::Completed));
            hub.publish_snapshot(final_snapshot);
        }

        let stats = self.finalize(start_time.elapsed());
        hub.publish_complete(stats.clone());
        stats
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
    spawn_collector_with_hub(receiver, counters, start_time, None)
}

/// Spawns a background Tokio task that collects [`MetricEvent`]s and optionally publishes periodic
/// windowed [`TelemetrySnapshot`]s to `telemetry_hub`.
pub fn spawn_collector_with_hub(
    receiver: UnboundedReceiver<MetricEvent>,
    counters: Arc<LiveCounters>,
    start_time: Instant,
    telemetry_hub: Option<Arc<TelemetryHub>>,
) -> JoinHandle<BenchmarkStats> {
    let mut collector =
        MetricsCollector::new(counters).expect("failed to initialize LatencyHistogram");
    if let Some(hub) = telemetry_hub {
        collector = collector
            .with_telemetry_hub(hub)
            .expect("failed to initialize windowed LatencyHistogram");
    }
    tokio::spawn(async move { collector.run(receiver, start_time).await })
}
