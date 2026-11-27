//! Single worker task loop executing HTTP requests and sampling high-resolution timings.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use reqwest::header::HeaderMap;
use reqwest::{Body, Client, Request};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::config::BenchmarkConfig;
use crate::metrics::event::{JourneyOutcome, MetricEvent, StepFailureKind};
use crate::metrics::snapshot::current_timestamp_ms;
use crate::metrics::stats::LiveCounters;
use crate::scenario::{
    BuiltInVars, CompiledScenario, CompiledStep, StepFailurePolicy, MAX_EXTRACTION_BODY_BYTES,
};

/// Asynchronous load-generation worker that continuously sends HTTP requests
/// using a shared [`reqwest::Client`] until cancelled.
#[derive(Debug, Clone)]
pub struct Worker {
    pub id: usize,
    pub client: Client,
    pub config: Arc<BenchmarkConfig>,
    pub sender: UnboundedSender<MetricEvent>,
    pub cancel_token: CancellationToken,
    pub counters: Option<Arc<LiveCounters>>,
}

impl Worker {
    /// Creates a new [`Worker`] instance.
    pub fn new(
        id: usize,
        client: Client,
        config: Arc<BenchmarkConfig>,
        sender: UnboundedSender<MetricEvent>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            id,
            client,
            config,
            sender,
            cancel_token,
            counters: None,
        }
    }

    /// Attaches shared [`LiveCounters`] for tracking active worker concurrency.
    pub fn with_counters(mut self, counters: Arc<LiveCounters>) -> Self {
        self.counters = Some(counters);
        self
    }

    /// Returns the worker's numeric identifier.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Constructs a fresh [`reqwest::Request`] from the shared [`BenchmarkConfig`].
    pub fn build_request(&self) -> Request {
        let mut request = Request::new(self.config.method.clone(), self.config.url.clone());

        if !self.config.headers.is_empty() {
            *request.headers_mut() = self.config.headers.clone();
        }

        if let Some(body) = &self.config.body {
            *request.body_mut() = Some(Body::from(body.clone()));
        }

        request
    }

    /// Executes a single HTTP request attempt, draining the full response body
    /// before computing end-to-end latency.
    pub async fn execute_once(&self) -> MetricEvent {
        let request = self.build_request();
        let start = Instant::now();

        match self.client.execute(request).await {
            Ok(mut response) => {
                let status = response.status();
                let mut total_len = 0usize;
                loop {
                    match response.chunk().await {
                        Ok(Some(chunk)) => {
                            total_len += chunk.len();
                            drop(chunk);
                        }
                        Ok(None) => {
                            let latency = start.elapsed();
                            drop(response);
                            return MetricEvent::response(latency, status, total_len);
                        }
                        Err(err) => {
                            let latency = start.elapsed();
                            drop(response);
                            return MetricEvent::error(latency, classify_reqwest_error(&err));
                        }
                    }
                }
            }
            Err(err) => {
                let latency = start.elapsed();
                MetricEvent::error(latency, classify_reqwest_error(&err))
            }
        }
    }

    /// Runs the continuous load generation loop until `cancel_token` is cancelled
    /// or the metrics receiver channel closes.
    pub async fn run(self) {
        let _active_guard = self.counters.as_ref().map(|c| c.track_worker());

        if let Some(scenario) = self.config.scenario.clone() {
            self.run_scenario(scenario).await;
            return;
        }

        loop {
            if self.cancel_token.is_cancelled() {
                break;
            }

            tokio::select! {
                biased;
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                event = self.execute_once() => {
                    if self.sender.send(event).is_err() {
                        break;
                    }
                }
            }

            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        }
    }

    async fn run_scenario(self, scenario: Arc<CompiledScenario>) {
        let mut vars = HashMap::with_capacity(scenario.initial_variables.len() + 8);
        let mut iteration = 0u64;

        loop {
            if self.cancel_token.is_cancelled() {
                break;
            }

            iteration += 1;
            vars.clear();
            for (name, val, _) in &scenario.initial_variables {
                vars.insert(name.clone(), val.clone());
            }

            let built_ins = BuiltInVars {
                vu_id: self.id,
                iteration,
                timestamp_ms: current_timestamp_ms(),
            };
            let iter_start = Instant::now();
            let step_count = scenario.steps.len();

            for (idx, step) in scenario.steps.iter().enumerate() {
                if self.cancel_token.is_cancelled() {
                    return;
                }

                let mut event = match step.build_request(&vars, &built_ins) {
                    Ok((request, _, _)) => {
                        tokio::select! {
                            biased;
                            _ = self.cancel_token.cancelled() => {
                                return;
                            }
                            ev = self.execute_scenario_step(step, request, &mut vars) => ev
                        }
                    }
                    Err(err) => MetricEvent::error(std::time::Duration::ZERO, err)
                        .with_step_index(step.step_index as u16),
                };

                let step_failed = event.error.is_some() || event.failure_kind.is_some();
                let is_last = idx + 1 == step_count;
                let should_abort = step_failed && step.on_failure == StepFailurePolicy::AbortIteration;

                if should_abort {
                    event = event.with_journey_outcome(JourneyOutcome::Aborted {
                        failed_step_index: step.step_index as u16,
                        duration: iter_start.elapsed(),
                    });
                } else if is_last {
                    event = event.with_journey_outcome(JourneyOutcome::Completed {
                        duration: iter_start.elapsed(),
                    });
                }

                if self.sender.send(event).is_err() {
                    return;
                }

                if should_abort {
                    break;
                }

                if !step.think_time.is_zero() {
                    tokio::select! {
                        biased;
                        _ = self.cancel_token.cancelled() => {
                            return;
                        }
                        _ = tokio::time::sleep(step.think_time) => {}
                    }
                }
            }

            tokio::task::yield_now().await;
        }
    }

    async fn execute_scenario_step(
        &self,
        step: &CompiledStep,
        request: Request,
        vars: &mut HashMap<String, String>,
    ) -> MetricEvent {
        let step_idx = step.step_index as u16;
        let start = Instant::now();

        match self.client.execute(request).await {
            Ok(mut response) => {
                let status = response.status();
                let resp_headers = if step.extractors.is_empty() {
                    HeaderMap::new()
                } else {
                    response.headers().clone()
                };

                let mut total_len = 0usize;
                if step.needs_body_buffer() {
                    let mut buf = Vec::new();
                    let mut exceeded = false;
                    loop {
                        match response.chunk().await {
                            Ok(Some(chunk)) => {
                                total_len += chunk.len();
                                if !exceeded {
                                    if buf.len().saturating_add(chunk.len())
                                        > MAX_EXTRACTION_BODY_BYTES
                                    {
                                        exceeded = true;
                                        buf.clear();
                                        buf.shrink_to_fit();
                                    } else {
                                        buf.extend_from_slice(&chunk);
                                    }
                                }
                                drop(chunk);
                            }
                            Ok(None) => break,
                            Err(err) => {
                                let latency = start.elapsed();
                                drop(response);
                                return MetricEvent::error(latency, classify_reqwest_error(&err))
                                    .with_step_index(step_idx);
                            }
                        }
                    }
                    let latency = start.elapsed();
                    drop(response);

                    if exceeded {
                        return MetricEvent::response(latency, status, total_len)
                            .with_step_index(step_idx)
                            .with_failure_kind(
                                StepFailureKind::ExtractionFailed,
                                "BodyExceedsExtractionLimit",
                            );
                    }

                    let eval = step.evaluate_response(
                        status,
                        &resp_headers,
                        Some(&buf),
                        vars,
                        false,
                    );
                    drop(buf);

                    let mut ev =
                        MetricEvent::response(latency, status, total_len).with_step_index(step_idx);
                    if let (Some(kind), Some(reason)) = (eval.failure_kind, eval.error_message) {
                        ev = ev.with_failure_kind(kind, reason);
                    }
                    ev
                } else {
                    loop {
                        match response.chunk().await {
                            Ok(Some(chunk)) => {
                                total_len += chunk.len();
                                drop(chunk);
                            }
                            Ok(None) => break,
                            Err(err) => {
                                let latency = start.elapsed();
                                drop(response);
                                return MetricEvent::error(latency, classify_reqwest_error(&err))
                                    .with_step_index(step_idx);
                            }
                        }
                    }
                    let latency = start.elapsed();
                    drop(response);

                    let eval = step.evaluate_response(status, &resp_headers, None, vars, false);
                    let mut ev =
                        MetricEvent::response(latency, status, total_len).with_step_index(step_idx);
                    if let (Some(kind), Some(reason)) = (eval.failure_kind, eval.error_message) {
                        ev = ev.with_failure_kind(kind, reason);
                    }
                    ev
                }
            }
            Err(err) => {
                let latency = start.elapsed();
                MetricEvent::error(latency, classify_reqwest_error(&err)).with_step_index(step_idx)
            }
        }
    }
}

/// Classifies a [`reqwest::Error`] into a concise, deterministic category label.
pub fn classify_reqwest_error(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "Timeout"
    } else if err.is_connect() {
        "ConnectError"
    } else if err.is_body() || err.is_decode() {
        "BodyReadError"
    } else if err.is_redirect() {
        "RedirectError"
    } else if err.is_request() {
        "RequestError"
    } else if err.is_builder() {
        "BuilderError"
    } else {
        "NetworkError"
    }
}
