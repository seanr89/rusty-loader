//! Multi-run benchmark lifecycle manager for interactive desktop and web UI sessions.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::{validate_input, BenchmarkConfig, BenchmarkConfigInput, ValidationReport};
use crate::scenario::{redact_scenario_input, RequestMode};
use crate::engine::{TestController, SHUTDOWN_GRACE_PERIOD};
use crate::gui::api::{
    build_partial_stats_from_parts, is_sensitive_header, meta_from_config, sanitize_url, GuiMode,
    MetaResponse,
};
use crate::metrics::snapshot::{
    current_timestamp_ms, TelemetryHub, TelemetryPhase, TelemetrySnapshot,
};
use crate::metrics::stats::{BenchmarkStats, LiveCounters};
use crate::reporter::{
    export_csv_report, export_json_report, export_pdf_report, ExportReport, JsonReport,
};

/// Maximum number of completed/failed runs retained in memory for a UI session.
pub const MAX_RUN_HISTORY: usize = 50;

const LIFECYCLE_BROADCAST_CAPACITY: usize = 32;

/// Full archived record for a completed or failed interactive benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedRunRecord {
    pub run_id: String,
    pub timestamp: String,
    pub preset_name: Option<String>,
    pub meta: MetaResponse,
    pub stats: Option<BenchmarkStats>,
    pub snapshots: Vec<TelemetrySnapshot>,
    pub error_message: Option<String>,
    pub config_redacted: BenchmarkConfigInput,
}

/// Lightweight summary row returned by `GET /api/runs` for session history and comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedRunSummary {
    pub run_id: String,
    pub timestamp: String,
    pub preset_name: Option<String>,
    pub url: String,
    pub method: String,
    pub concurrency: usize,
    pub phase: TelemetryPhase,
    pub total_requests: u64,
    pub requests_per_sec: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub error_rate_pct: f64,
    pub total_duration_secs: f64,
    pub error_message: Option<String>,
}

/// Events broadcast to connected SSE clients across interactive run transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunLifecycleEvent {
    RunStarted {
        run_id: String,
        meta: MetaResponse,
    },
    RunCompleted {
        run_id: String,
        stats: BenchmarkStats,
        meta: MetaResponse,
    },
    RunFailed {
        run_id: String,
        error: String,
        meta: MetaResponse,
    },
}

/// Errors returned by [`RunManager`] control operations.
#[derive(Debug, Error)]
pub enum RunManagerError {
    #[error("a benchmark run is already in progress")]
    RunAlreadyActive,
    #[error("no active benchmark run to stop")]
    NoActiveRun,
    #[error("invalid benchmark configuration")]
    InvalidConfig(ValidationReport),
    #[error("{0}")]
    Internal(String),
}

struct ActiveRunState {
    run_id: String,
    preset_name: Option<String>,
    config: Arc<BenchmarkConfig>,
    hub: Arc<TelemetryHub>,
    counters: Arc<LiveCounters>,
    cancel_token: CancellationToken,
    started_at: Instant,
    task_handle: Option<JoinHandle<()>>,
}

struct ManagerInner {
    active: Option<ActiveRunState>,
    history: VecDeque<CompletedRunRecord>,
}

/// Thread-safe coordinator managing sequential interactive benchmark runs.
pub struct RunManager {
    inner: Arc<RwLock<ManagerInner>>,
    lifecycle_tx: broadcast::Sender<RunLifecycleEvent>,
    seq: AtomicU64,
}

impl Default for RunManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RunManager {
    /// Creates an idle [`RunManager`] with an empty session run history.
    pub fn new() -> Self {
        let (lifecycle_tx, _) = broadcast::channel(LIFECYCLE_BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(RwLock::new(ManagerInner {
                active: None,
                history: VecDeque::with_capacity(MAX_RUN_HISTORY),
            })),
            lifecycle_tx,
            seq: AtomicU64::new(1),
        }
    }

    /// Validates `input`, builds a [`TestController`], and launches a new benchmark run in the background.
    pub fn start_run(
        &self,
        input: BenchmarkConfigInput,
        preset_name: Option<String>,
    ) -> Result<(String, MetaResponse), RunManagerError> {
        self.start_run_with_tick_interval(input, preset_name, None)
    }

    /// Launches a new benchmark run with an optional custom snapshot interval (primarily for tests).
    pub fn start_run_with_tick_interval(
        &self,
        input: BenchmarkConfigInput,
        preset_name: Option<String>,
        tick_interval: Option<Duration>,
    ) -> Result<(String, MetaResponse), RunManagerError> {
        let report = validate_input(&input, None);
        if !report.valid {
            return Err(RunManagerError::InvalidConfig(report));
        }

        let sanitized_input = redact_config_input(&input);
        let config = BenchmarkConfig::from_input(input).map_err(|err| {
            let mut field_errors = HashMap::new();
            field_errors.insert("config".to_string(), err.to_string());
            RunManagerError::InvalidConfig(ValidationReport {
                valid: false,
                field_errors,
                warnings: Vec::new(),
            })
        })?;

        let hub = match tick_interval {
            Some(interval) => Arc::new(TelemetryHub::with_options(
                config.duration,
                config.ramp_up,
                interval,
                crate::metrics::snapshot::DEFAULT_HISTORY_CAPACITY,
                crate::metrics::snapshot::DEFAULT_BROADCAST_CAPACITY,
            )),
            None => Arc::new(TelemetryHub::new(config.duration, config.ramp_up)),
        };

        let controller = TestController::from_config(config)
            .map_err(|err| RunManagerError::Internal(err.to_string()))?
            .with_signal_handler(false)
            .with_telemetry_hub(Arc::clone(&hub));

        let config_arc = Arc::clone(controller.config());
        let counters = controller.counters();
        let cancel_token = controller.cancel_token();
        let started_at = Instant::now();

        let seq_num = self.seq.fetch_add(1, Ordering::Relaxed);
        let run_id = format!("run-{}-{seq_num}", current_timestamp_ms());
        let meta = meta_from_config(
            &config_arc,
            hub.current_phase(Duration::ZERO),
            GuiMode::Interactive,
            Some(run_id.clone()),
            preset_name.clone(),
            None,
        );

        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if guard.active.is_some() {
            return Err(RunManagerError::RunAlreadyActive);
        }

        let run_id_task = run_id.clone();
        let inner_ref = Arc::clone(&self.inner);
        let lifecycle_tx = self.lifecycle_tx.clone();
        let hub_task = Arc::clone(&hub);
        let config_task = Arc::clone(&config_arc);
        let preset_task = preset_name.clone();
        let sanitized_task = sanitized_input.clone();

        let task_handle = tokio::spawn(async move {
            let result = controller.run().await;
            let snapshots: Vec<TelemetrySnapshot> =
                hub_task.history().into_iter().map(|s| (*s).clone()).collect();

            if let Ok(ref stats) = result {
                if let Some(ref output_path) = config_task.output_json {
                    if let Err(err) = export_json_report(output_path, &config_task, stats) {
                        eprintln!(
                            "Warning: failed to write JSON report to '{}': {err}",
                            output_path.display()
                        );
                    }
                }
                if config_task.output_csv.is_some() || config_task.output_pdf.is_some() {
                    let export = ExportReport::from_config_redacted(
                        &config_task,
                        stats,
                        snapshots.clone(),
                        preset_task.clone(),
                    );
                    if let Some(ref csv_path) = config_task.output_csv {
                        if let Err(err) = export_csv_report(csv_path, &export) {
                            eprintln!(
                                "Warning: failed to write CSV report to '{}': {err}",
                                csv_path.display()
                            );
                        }
                    }
                    if let Some(ref pdf_path) = config_task.output_pdf {
                        if let Err(err) = export_pdf_report(pdf_path, &export) {
                            eprintln!(
                                "Warning: failed to write PDF report to '{}': {err}",
                                pdf_path.display()
                            );
                        }
                    }
                }
            }

            let mut guard = inner_ref
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = guard.active.take();

            match result {
                Ok(stats) => {
                    let final_meta = meta_from_config(
                        &config_task,
                        TelemetryPhase::Completed,
                        GuiMode::Interactive,
                        Some(run_id_task.clone()),
                        preset_task.clone(),
                        None,
                    );
                    let record = CompletedRunRecord {
                        run_id: run_id_task.clone(),
                        timestamp: Utc::now().to_rfc3339(),
                        preset_name: preset_task,
                        meta: final_meta.clone(),
                        stats: Some(stats.clone()),
                        snapshots,
                        error_message: None,
                        config_redacted: sanitized_task,
                    };
                    push_history(&mut guard.history, record);
                    let _ = lifecycle_tx.send(RunLifecycleEvent::RunCompleted {
                        run_id: run_id_task,
                        stats,
                        meta: final_meta,
                    });
                }
                Err(err) => {
                    let msg = err.to_string();
                    let failed_meta = meta_from_config(
                        &config_task,
                        TelemetryPhase::Failed,
                        GuiMode::Interactive,
                        Some(run_id_task.clone()),
                        preset_task.clone(),
                        Some(msg.clone()),
                    );
                    let record = CompletedRunRecord {
                        run_id: run_id_task.clone(),
                        timestamp: Utc::now().to_rfc3339(),
                        preset_name: preset_task,
                        meta: failed_meta.clone(),
                        stats: None,
                        snapshots,
                        error_message: Some(msg.clone()),
                        config_redacted: sanitized_task,
                    };
                    push_history(&mut guard.history, record);
                    let _ = lifecycle_tx.send(RunLifecycleEvent::RunFailed {
                        run_id: run_id_task,
                        error: msg,
                        meta: failed_meta,
                    });
                }
            }
        });

        guard.active = Some(ActiveRunState {
            run_id: run_id.clone(),
            preset_name,
            config: config_arc,
            hub,
            counters,
            cancel_token,
            started_at,
            task_handle: Some(task_handle),
        });

        // Send RunStarted while holding the write lock so subscribers always observe RunStarted
        // before the spawned task can acquire the write lock and emit RunCompleted/RunFailed.
        let _ = self.lifecycle_tx.send(RunLifecycleEvent::RunStarted {
            run_id: run_id.clone(),
            meta: meta.clone(),
        });

        Ok((run_id, meta))
    }

    /// Gracefully stops the currently active benchmark run, returning its `run_id`.
    pub fn stop_run(&self) -> Result<String, RunManagerError> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(ref active) = guard.active else {
            return Err(RunManagerError::NoActiveRun);
        };
        active.hub.mark_draining();
        active.cancel_token.cancel();
        Ok(active.run_id.clone())
    }

    /// Returns the current [`MetaResponse`] reflecting active, latest completed, or idle state.
    pub fn current_meta(&self) -> MetaResponse {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(ref active) = guard.active {
            let phase = active.hub.current_phase(active.started_at.elapsed());
            return meta_from_config(
                &active.config,
                phase,
                GuiMode::Interactive,
                Some(active.run_id.clone()),
                active.preset_name.clone(),
                None,
            );
        }
        if let Some(latest) = guard.history.front() {
            return latest.meta.clone();
        }
        idle_meta()
    }

    /// Returns handles to the currently active run's telemetry hub and counters, if a run is active.
    pub fn active_telemetry(&self) -> Option<(String, Arc<TelemetryHub>, Arc<LiveCounters>)> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.active.as_ref().map(|active| {
            (
                active.run_id.clone(),
                Arc::clone(&active.hub),
                Arc::clone(&active.counters),
            )
        })
    }

    /// Returns chronological [`TelemetrySnapshot`] history for `run_id`, active run, or latest run.
    pub fn current_history(&self, run_id: Option<&str>) -> Vec<TelemetrySnapshot> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(target_id) = run_id {
            if let Some(ref active) = guard.active {
                if active.run_id == target_id {
                    return active.hub.history().into_iter().map(|s| (*s).clone()).collect();
                }
            }
            if let Some(record) = guard.history.iter().find(|r| r.run_id == target_id) {
                return record.snapshots.clone();
            }
            return Vec::new();
        }

        if let Some(ref active) = guard.active {
            return active.hub.history().into_iter().map(|s| (*s).clone()).collect();
        }

        guard
            .history
            .front()
            .map(|r| r.snapshots.clone())
            .unwrap_or_default()
    }

    /// Builds a redacted [`ExportReport`] for `run_id`, active run, or latest completed run.
    pub fn export_report_data(&self, run_id: Option<&str>) -> Option<ExportReport> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(target_id) = run_id {
            if let Some(ref active) = guard.active {
                if active.run_id == target_id {
                    return Some(export_for_active(active));
                }
            }
            return guard
                .history
                .iter()
                .find(|r| r.run_id == target_id)
                .and_then(ExportReport::from_completed_record);
        }

        if let Some(ref active) = guard.active {
            return Some(export_for_active(active));
        }

        guard
            .history
            .front()
            .and_then(ExportReport::from_completed_record)
    }

    /// Builds a redacted [`JsonReport`] for the requested `run_id`, active run, or latest completed run.
    pub fn export_report(&self, run_id: Option<&str>) -> Option<JsonReport> {
        self.export_report_data(run_id)
            .map(|report| report.to_json_report())
    }

    /// Returns summaries of all recorded runs in reverse chronological order (newest first).
    pub fn list_runs(&self) -> Vec<CompletedRunSummary> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.history.iter().map(summarize_record).collect()
    }

    /// Returns the full archived [`CompletedRunRecord`] for `run_id`, if present.
    pub fn get_run(&self, run_id: &str) -> Option<CompletedRunRecord> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .history
            .iter()
            .find(|r| r.run_id == run_id)
            .cloned()
    }

    /// Subscribes to run start/completion/failure lifecycle events.
    pub fn subscribe_lifecycle(&self) -> broadcast::Receiver<RunLifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    /// Cancels any in-flight run and waits up to [`SHUTDOWN_GRACE_PERIOD`] + 1s for drain completion.
    pub async fn shutdown(&self) {
        let handle_opt = {
            let mut guard = self
                .inner
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(ref mut active) = guard.active {
                active.hub.mark_draining();
                active.cancel_token.cancel();
                active.task_handle.take()
            } else {
                None
            }
        };

        if let Some(mut handle) = handle_opt {
            let wait_dur = SHUTDOWN_GRACE_PERIOD + Duration::from_secs(1);
            if tokio::time::timeout(wait_dur, &mut handle).await.is_err() {
                handle.abort();
                let _ = handle.await;
            }
        }
    }
}

/// Redacts sensitive header values and embedded URL userinfo in a [`BenchmarkConfigInput`].
pub fn redact_config_input(input: &BenchmarkConfigInput) -> BenchmarkConfigInput {
    let mut redacted = input.clone();
    if let Ok(parsed) = Url::parse(input.url.trim()) {
        redacted.url = sanitize_url(&parsed);
    }
    for header in &mut redacted.headers {
        if is_sensitive_header(&header.name) && !header.value.is_empty() {
            header.value = "[REDACTED]".to_string();
        }
    }
    if let Some(ref sc) = input.scenario {
        redacted.scenario = Some(redact_scenario_input(sc));
    }
    redacted
}

fn idle_meta() -> MetaResponse {
    MetaResponse {
        url: String::new(),
        method: "GET".to_string(),
        concurrency: 50,
        duration_secs: 30.0,
        ramp_up_secs: 0.0,
        timeout_ms: 5000,
        headers: HashMap::new(),
        has_body: false,
        body_size_bytes: 0,
        phase: TelemetryPhase::Idle,
        mode: GuiMode::Interactive,
        request_mode: RequestMode::Single,
        scenario: None,
        run_id: None,
        preset_name: None,
        error_message: None,
    }
}

fn push_history(history: &mut VecDeque<CompletedRunRecord>, record: CompletedRunRecord) {
    while history.len() >= MAX_RUN_HISTORY {
        history.pop_back();
    }
    history.push_front(record);
}

fn export_for_active(active: &ActiveRunState) -> ExportReport {
    let stats = match active.hub.final_stats() {
        Some(final_stats) => (*final_stats).clone(),
        None => build_partial_stats_from_parts(&active.hub, &active.counters, active.started_at),
    };
    let snapshots = active
        .hub
        .history()
        .into_iter()
        .map(|s| (*s).clone())
        .collect();
    ExportReport::from_config_redacted(
        &active.config,
        &stats,
        snapshots,
        active.preset_name.clone(),
    )
}

fn summarize_record(record: &CompletedRunRecord) -> CompletedRunSummary {
    match record.stats {
        Some(ref stats) => {
            let total_errors =
                stats.client_errors + stats.server_errors + stats.transport_errors;
            let error_rate_pct = if stats.total_requests > 0 {
                (total_errors as f64 / stats.total_requests as f64) * 100.0
            } else {
                0.0
            };
            CompletedRunSummary {
                run_id: record.run_id.clone(),
                timestamp: record.timestamp.clone(),
                preset_name: record.preset_name.clone(),
                url: record.meta.url.clone(),
                method: record.meta.method.clone(),
                concurrency: record.meta.concurrency,
                phase: record.meta.phase,
                total_requests: stats.total_requests,
                requests_per_sec: stats.requests_per_sec,
                p50_ms: stats.latencies.p50_ms,
                p95_ms: stats.latencies.p95_ms,
                p99_ms: stats.latencies.p99_ms,
                error_rate_pct,
                total_duration_secs: stats.total_duration_secs,
                error_message: record.error_message.clone(),
            }
        }
        None => CompletedRunSummary {
            run_id: record.run_id.clone(),
            timestamp: record.timestamp.clone(),
            preset_name: record.preset_name.clone(),
            url: record.meta.url.clone(),
            method: record.meta.method.clone(),
            concurrency: record.meta.concurrency,
            phase: record.meta.phase,
            total_requests: 0,
            requests_per_sec: 0.0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
            error_rate_pct: 0.0,
            total_duration_secs: 0.0,
            error_message: record.error_message.clone(),
        },
    }
}
