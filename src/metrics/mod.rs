//! High-fidelity metrics aggregation and HDR histogram statistics.

pub mod event;
pub mod histogram;
pub mod snapshot;
pub mod stats;

use crate::metrics::event::{JourneyOutcome, StepFailureKind};
use crate::metrics::stats::{JourneyStats, ScenarioStats, StepStats};
use crate::scenario::ScenarioStepMeta;

pub use event::MetricEvent;
pub use histogram::LatencyHistogram;
pub use snapshot::{
    current_timestamp_ms, JourneyTelemetrySnapshot, StepTelemetrySnapshot, TelemetryHub,
    TelemetryPhase, TelemetrySnapshot, WindowLatencyMs,
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
    scenario_state: Option<ScenarioMetricsState>,
}

struct StepMetricsState {
    meta: ScenarioStepMeta,
    histogram: LatencyHistogram,
    window_histogram: Option<LatencyHistogram>,
    total_requests: u64,
    successful_requests: u64,
    client_errors: u64,
    server_errors: u64,
    other_statuses: u64,
    transport_errors: u64,
    assertion_failures: u64,
    extraction_failures: u64,
    aborted_iterations: u64,
    status_distribution: HashMap<u16, u64>,
    error_breakdown: HashMap<String, u64>,
}

struct ScenarioMetricsState {
    steps: Vec<StepMetricsState>,
    journey_histogram: LatencyHistogram,
    journey_window_histogram: Option<LatencyHistogram>,
    completed_iterations: u64,
    aborted_iterations: u64,
    aborted_by_step: HashMap<usize, u64>,
}

impl ScenarioMetricsState {
    fn new(step_metas: Vec<ScenarioStepMeta>, with_window: bool) -> Result<Self, hdrhistogram::CreationError> {
        let mut steps = Vec::with_capacity(step_metas.len());
        for meta in step_metas {
            steps.push(StepMetricsState {
                meta,
                histogram: LatencyHistogram::new()?,
                window_histogram: if with_window {
                    Some(LatencyHistogram::new()?)
                } else {
                    None
                },
                total_requests: 0,
                successful_requests: 0,
                client_errors: 0,
                server_errors: 0,
                other_statuses: 0,
                transport_errors: 0,
                assertion_failures: 0,
                extraction_failures: 0,
                aborted_iterations: 0,
                status_distribution: HashMap::new(),
                error_breakdown: HashMap::new(),
            });
        }
        Ok(Self {
            steps,
            journey_histogram: LatencyHistogram::new()?,
            journey_window_histogram: if with_window {
                Some(LatencyHistogram::new()?)
            } else {
                None
            },
            completed_iterations: 0,
            aborted_iterations: 0,
            aborted_by_step: HashMap::new(),
        })
    }

    fn enable_windows(&mut self) -> Result<(), hdrhistogram::CreationError> {
        if self.journey_window_histogram.is_none() {
            self.journey_window_histogram = Some(LatencyHistogram::new()?);
        }
        for step in &mut self.steps {
            if step.window_histogram.is_none() {
                step.window_histogram = Some(LatencyHistogram::new()?);
            }
        }
        Ok(())
    }

    fn record(&mut self, event: &MetricEvent) {
        if let Some(step_idx) = event.step_index {
            if let Some(step) = self.steps.get_mut(step_idx as usize) {
                step.total_requests += 1;
                step.histogram.record(event.latency);
                if let Some(ref mut win) = step.window_histogram {
                    win.record(event.latency);
                }

                match event.status_code {
                    Some(status) => {
                        let code = status.as_u16();
                        *step.status_distribution.entry(code).or_insert(0) += 1;
                        if event.failure_kind.is_none() && (200..300).contains(&code) {
                            step.successful_requests += 1;
                        } else if (400..500).contains(&code) {
                            step.client_errors += 1;
                        } else if (500..600).contains(&code) {
                            step.server_errors += 1;
                        } else if event.failure_kind.is_none() {
                            step.other_statuses += 1;
                        }
                    }
                    None => {
                        step.transport_errors += 1;
                    }
                }

                match event.failure_kind {
                    Some(StepFailureKind::AssertionFailed) => step.assertion_failures += 1,
                    Some(StepFailureKind::ExtractionFailed) => step.extraction_failures += 1,
                    None => {}
                }

                if let Some(ref err) = event.error {
                    *step.error_breakdown.entry(err.clone()).or_insert(0) += 1;
                } else if event.status_code.is_none() {
                    *step
                        .error_breakdown
                        .entry("UnknownTransportError".to_string())
                        .or_insert(0) += 1;
                }
            }
        }

        if let Some(outcome) = event.journey_outcome {
            match outcome {
                JourneyOutcome::Completed { duration } => {
                    self.completed_iterations += 1;
                    self.journey_histogram.record(duration);
                    if let Some(ref mut win) = self.journey_window_histogram {
                        win.record(duration);
                    }
                }
                JourneyOutcome::Aborted {
                    failed_step_index,
                    duration,
                } => {
                    self.aborted_iterations += 1;
                    let idx = failed_step_index as usize;
                    *self.aborted_by_step.entry(idx).or_insert(0) += 1;
                    if let Some(step) = self.steps.get_mut(idx) {
                        step.aborted_iterations += 1;
                    }
                    self.journey_histogram.record(duration);
                    if let Some(ref mut win) = self.journey_window_histogram {
                        win.record(duration);
                    }
                }
            }
        }
    }

    fn take_window_snapshots(
        &mut self,
        elapsed_secs: f64,
    ) -> (Vec<StepTelemetrySnapshot>, JourneyTelemetrySnapshot) {
        let steps = self
            .steps
            .iter_mut()
            .map(|step| {
                let win_lat = step
                    .window_histogram
                    .as_mut()
                    .map(|h| {
                        let d = WindowLatencyMs::from(h.to_distribution());
                        h.reset();
                        d
                    })
                    .unwrap_or_default();
                let error_count = step.total_requests.saturating_sub(
                    step.successful_requests + step.other_statuses,
                );
                let avg_rps = if elapsed_secs > 0.0 {
                    step.total_requests as f64 / elapsed_secs
                } else {
                    0.0
                };
                StepTelemetrySnapshot {
                    step_index: step.meta.step_index,
                    order_number: step.meta.order_number,
                    step_id: step.meta.id.clone(),
                    step_name: step.meta.name.clone(),
                    method: step.meta.method.clone(),
                    target_preview: step.meta.target_preview.clone(),
                    total_requests: step.total_requests,
                    successful_requests: step.successful_requests,
                    error_count,
                    aborted_iterations: step.aborted_iterations,
                    avg_rps,
                    window_latency_ms: win_lat,
                }
            })
            .collect();

        let started_iterations = self.completed_iterations + self.aborted_iterations;
        let completion_rate_pct = if started_iterations > 0 {
            (self.completed_iterations as f64 / started_iterations as f64) * 100.0
        } else {
            100.0
        };
        let iterations_per_sec = if elapsed_secs > 0.0 {
            self.completed_iterations as f64 / elapsed_secs
        } else {
            0.0
        };
        let journey_win_lat = self
            .journey_window_histogram
            .as_mut()
            .map(|h| {
                let d = WindowLatencyMs::from(h.to_distribution());
                h.reset();
                d
            })
            .unwrap_or_default();

        let journey = JourneyTelemetrySnapshot {
            started_iterations,
            completed_iterations: self.completed_iterations,
            aborted_iterations: self.aborted_iterations,
            completion_rate_pct,
            iterations_per_sec,
            window_duration_ms: journey_win_lat,
        };

        (steps, journey)
    }

    fn finalize(self, total_duration_secs: f64) -> ScenarioStats {
        let steps = self
            .steps
            .into_iter()
            .map(|step| {
                let rps = if total_duration_secs > 0.0 {
                    step.total_requests as f64 / total_duration_secs
                } else {
                    0.0
                };
                StepStats {
                    step_index: step.meta.step_index,
                    order_number: step.meta.order_number,
                    step_id: step.meta.id,
                    step_name: step.meta.name,
                    method: step.meta.method,
                    target_preview: step.meta.target_preview,
                    on_failure: step.meta.on_failure,
                    total_requests: step.total_requests,
                    successful_requests: step.successful_requests,
                    client_errors: step.client_errors,
                    server_errors: step.server_errors,
                    other_statuses: step.other_statuses,
                    transport_errors: step.transport_errors,
                    assertion_failures: step.assertion_failures,
                    extraction_failures: step.extraction_failures,
                    aborted_iterations: step.aborted_iterations,
                    requests_per_sec: rps,
                    latencies: step.histogram.to_distribution(),
                    status_distribution: step.status_distribution,
                    error_breakdown: step.error_breakdown,
                }
            })
            .collect();

        let started_iterations = self.completed_iterations + self.aborted_iterations;
        let completion_rate_pct = if started_iterations > 0 {
            (self.completed_iterations as f64 / started_iterations as f64) * 100.0
        } else {
            0.0
        };
        let iterations_per_sec = if total_duration_secs > 0.0 {
            self.completed_iterations as f64 / total_duration_secs
        } else {
            0.0
        };

        ScenarioStats {
            steps,
            journeys: JourneyStats {
                started_iterations,
                completed_iterations: self.completed_iterations,
                aborted_iterations: self.aborted_iterations,
                completion_rate_pct,
                iterations_per_sec,
                aborted_by_step: self.aborted_by_step,
                duration: self.journey_histogram.to_distribution(),
            },
        }
    }
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
            scenario_state: None,
        })
    }

    /// Initializes per-step and journey accumulators for ordered multi-step sequence runs.
    pub fn with_scenario_steps(
        mut self,
        steps: Vec<ScenarioStepMeta>,
    ) -> Result<Self, hdrhistogram::CreationError> {
        if !steps.is_empty() {
            let with_window = self.window_histogram.is_some();
            self.scenario_state = Some(ScenarioMetricsState::new(steps, with_window)?);
        }
        Ok(self)
    }

    /// Attaches a [`TelemetryHub`] and initializes the windowed [`LatencyHistogram`]
    /// for periodic real-time snapshot emission.
    pub fn with_telemetry_hub(
        mut self,
        hub: Arc<TelemetryHub>,
    ) -> Result<Self, hdrhistogram::CreationError> {
        self.window_histogram = Some(LatencyHistogram::new()?);
        if let Some(ref mut sc) = self.scenario_state {
            sc.enable_windows()?;
        }
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

        if let Some(ref mut sc) = self.scenario_state {
            sc.record(&event);
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

        let (step_snapshots, journey_snapshot) = match self.scenario_state.as_mut() {
            Some(sc) => {
                let (steps, journey) = sc.take_window_snapshots(elapsed_secs);
                (Some(steps), Some(journey))
            }
            None => (None, None),
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
            step_snapshots,
            journey_snapshot,
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

        let scenario_stats = self
            .scenario_state
            .map(|sc| sc.finalize(total_duration_secs));

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
            scenario_stats,
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
    spawn_collector_with_options(receiver, counters, start_time, None, None)
}

/// Spawns a background Tokio task that collects [`MetricEvent`]s and optionally publishes periodic
/// windowed [`TelemetrySnapshot`]s to `telemetry_hub`.
pub fn spawn_collector_with_hub(
    receiver: UnboundedReceiver<MetricEvent>,
    counters: Arc<LiveCounters>,
    start_time: Instant,
    telemetry_hub: Option<Arc<TelemetryHub>>,
) -> JoinHandle<BenchmarkStats> {
    spawn_collector_with_options(receiver, counters, start_time, telemetry_hub, None)
}

/// Spawns a background Tokio task with optional [`TelemetryHub`] and ordered sequence step metadata.
pub fn spawn_collector_with_options(
    receiver: UnboundedReceiver<MetricEvent>,
    counters: Arc<LiveCounters>,
    start_time: Instant,
    telemetry_hub: Option<Arc<TelemetryHub>>,
    scenario_steps: Option<Vec<ScenarioStepMeta>>,
) -> JoinHandle<BenchmarkStats> {
    let mut collector =
        MetricsCollector::new(counters).expect("failed to initialize LatencyHistogram");
    if let Some(steps) = scenario_steps {
        collector = collector
            .with_scenario_steps(steps)
            .expect("failed to initialize scenario step histograms");
    }
    if let Some(hub) = telemetry_hub {
        collector = collector
            .with_telemetry_hub(hub)
            .expect("failed to initialize windowed LatencyHistogram");
    }
    tokio::spawn(async move { collector.run(receiver, start_time).await })
}
