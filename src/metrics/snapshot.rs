//! Real-time windowed telemetry snapshots and non-blocking broadcast hub.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::metrics::stats::{BenchmarkStats, LatencyDistribution};

/// Default interval between real-time telemetry snapshots (250 ms).
pub const DEFAULT_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(250);

/// Default number of historical telemetry snapshots retained for late/reconnected clients.
pub const DEFAULT_HISTORY_CAPACITY: usize = 3_600;

/// Default capacity for the non-blocking telemetry broadcast channel.
pub const DEFAULT_BROADCAST_CAPACITY: usize = 64;

/// Lifecycle phase of an active, idle, completed, or failed benchmark run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryPhase {
    Idle,
    RampingUp,
    Running,
    Draining,
    Completed,
    Failed,
}

/// Windowed latency percentile summary in milliseconds for a single telemetry interval.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowLatencyMs {
    pub min: f64,
    pub mean: f64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl From<LatencyDistribution> for WindowLatencyMs {
    fn from(dist: LatencyDistribution) -> Self {
        Self {
            min: dist.min_ms,
            mean: dist.mean_ms,
            p50: dist.p50_ms,
            p90: dist.p90_ms,
            p95: dist.p95_ms,
            p99: dist.p99_ms,
            max: dist.max_ms,
        }
    }
}

/// Point-in-time telemetry snapshot streamed to live UI clients and retained in history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    pub timestamp_ms: u64,
    pub elapsed_secs: f64,
    pub duration_secs: f64,
    pub phase: TelemetryPhase,
    pub active_workers: usize,
    pub total_requests: u64,
    pub window_rps: f64,
    pub avg_rps: f64,
    pub bytes_per_sec: f64,
    pub total_bytes: u64,
    pub successful_requests: u64,
    pub client_errors: u64,
    pub server_errors: u64,
    pub other_statuses: u64,
    pub transport_errors: u64,
    pub window_latency_ms: WindowLatencyMs,
    pub status_distribution: HashMap<u16, u64>,
    pub error_breakdown: HashMap<String, u64>,
}

/// Returns the current UNIX epoch timestamp in milliseconds.
pub fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Thread-safe, non-blocking hub broadcasting live [`TelemetrySnapshot`]s and
/// storing a bounded history ring buffer plus the finalized [`BenchmarkStats`].
#[derive(Debug)]
pub struct TelemetryHub {
    configured_duration: Duration,
    ramp_up_duration: Duration,
    tick_interval: Duration,
    history_capacity: usize,
    sender: broadcast::Sender<Arc<TelemetrySnapshot>>,
    history: RwLock<VecDeque<Arc<TelemetrySnapshot>>>,
    completion_tx: watch::Sender<Option<Arc<BenchmarkStats>>>,
    completion_rx: watch::Receiver<Option<Arc<BenchmarkStats>>>,
    draining: AtomicBool,
}

impl TelemetryHub {
    /// Creates a new [`TelemetryHub`] with default interval (`250ms`), history (`3600`),
    /// and broadcast channel capacity (`64`).
    pub fn new(configured_duration: Duration, ramp_up_duration: Duration) -> Self {
        Self::with_options(
            configured_duration,
            ramp_up_duration,
            DEFAULT_SNAPSHOT_INTERVAL,
            DEFAULT_HISTORY_CAPACITY,
            DEFAULT_BROADCAST_CAPACITY,
        )
    }

    /// Creates a new [`TelemetryHub`] with custom tick interval, history capacity, and broadcast capacity.
    pub fn with_options(
        configured_duration: Duration,
        ramp_up_duration: Duration,
        tick_interval: Duration,
        history_capacity: usize,
        broadcast_capacity: usize,
    ) -> Self {
        let (sender, _) = broadcast::channel(broadcast_capacity.max(1));
        let (completion_tx, completion_rx) = watch::channel(None);

        Self {
            configured_duration,
            ramp_up_duration,
            tick_interval: if tick_interval.is_zero() {
                DEFAULT_SNAPSHOT_INTERVAL
            } else {
                tick_interval
            },
            history_capacity,
            sender,
            history: RwLock::new(VecDeque::with_capacity(history_capacity.min(4_096))),
            completion_tx,
            completion_rx,
            draining: AtomicBool::new(false),
        }
    }

    /// Returns the snapshot emission interval.
    pub fn tick_interval(&self) -> Duration {
        self.tick_interval
    }

    /// Returns the configured benchmark duration.
    pub fn configured_duration(&self) -> Duration {
        self.configured_duration
    }

    /// Returns the configured ramp-up duration.
    pub fn ramp_up_duration(&self) -> Duration {
        self.ramp_up_duration
    }

    /// Marks the benchmark as entering graceful teardown / draining.
    pub fn mark_draining(&self) {
        self.draining.store(true, Ordering::Release);
    }

    /// Returns `true` if the benchmark has begun draining.
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    /// Determines the current [`TelemetryPhase`] for a given elapsed duration.
    pub fn current_phase(&self, elapsed: Duration) -> TelemetryPhase {
        if self.final_stats().is_some() {
            TelemetryPhase::Completed
        } else if self.is_draining() {
            TelemetryPhase::Draining
        } else if !self.ramp_up_duration.is_zero() && elapsed < self.ramp_up_duration {
            TelemetryPhase::RampingUp
        } else {
            TelemetryPhase::Running
        }
    }

    /// Appends `snapshot` to the bounded ring buffer and broadcasts it to active subscribers.
    ///
    /// Never blocks or fails even if zero receivers are connected or slow receivers lag behind.
    pub fn publish_snapshot(&self, snapshot: TelemetrySnapshot) -> Arc<TelemetrySnapshot> {
        let arc = Arc::new(snapshot);

        if self.history_capacity > 0 {
            let mut guard = self
                .history
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while guard.len() >= self.history_capacity {
                guard.pop_front();
            }
            guard.push_back(Arc::clone(&arc));
        }

        let _ = self.sender.send(Arc::clone(&arc));
        arc
    }

    /// Stores and broadcasts the final [`BenchmarkStats`] once the run completes.
    pub fn publish_complete(&self, stats: BenchmarkStats) -> Arc<BenchmarkStats> {
        let arc = Arc::new(stats);
        let _ = self.completion_tx.send(Some(Arc::clone(&arc)));
        arc
    }

    /// Subscribes to real-time [`TelemetrySnapshot`] updates.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<TelemetrySnapshot>> {
        self.sender.subscribe()
    }

    /// Subscribes to the benchmark completion signal carrying the final [`BenchmarkStats`].
    pub fn subscribe_completion(&self) -> watch::Receiver<Option<Arc<BenchmarkStats>>> {
        self.completion_rx.clone()
    }

    /// Returns all snapshots currently stored in the history ring buffer in chronological order.
    pub fn history(&self) -> Vec<Arc<TelemetrySnapshot>> {
        let guard = self
            .history
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.iter().cloned().collect()
    }

    /// Returns the most recently published [`TelemetrySnapshot`], if any.
    pub fn latest_snapshot(&self) -> Option<Arc<TelemetrySnapshot>> {
        let guard = self
            .history
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.back().cloned()
    }

    /// Returns the finalized [`BenchmarkStats`] once the run has completed, or `None` while running.
    pub fn final_stats(&self) -> Option<Arc<BenchmarkStats>> {
        self.completion_rx.borrow().clone()
    }
}
