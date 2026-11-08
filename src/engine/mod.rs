//! Async worker pool, ramp-up orchestration, and signal handling.

pub mod signals;
pub mod spawner;
pub mod worker;

pub use worker::Worker;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reqwest::Client;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::client::HttpClientFactory;
use crate::config::BenchmarkConfig;
use crate::metrics::event::MetricEvent;
use crate::metrics::stats::{BenchmarkStats, LiveCounters};
use crate::metrics::spawn_collector;

/// Maximum grace period to wait for in-flight workers to finish after cancellation.
pub const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(3);

/// Coordinates the complete benchmark lifecycle: channel setup, metrics collection,
/// worker pool spawning (with optional ramp-up), duration/signal monitoring, and graceful teardown.
pub struct TestController {
    config: Arc<BenchmarkConfig>,
    client: Client,
    cancel_token: CancellationToken,
    counters: Arc<LiveCounters>,
}

impl TestController {
    /// Creates a new [`TestController`] from a shared [`BenchmarkConfig`] and [`reqwest::Client`].
    pub fn new(config: Arc<BenchmarkConfig>, client: Client) -> Self {
        Self {
            config,
            client,
            cancel_token: CancellationToken::new(),
            counters: Arc::new(LiveCounters::new()),
        }
    }

    /// Builds a [`reqwest::Client`] via [`HttpClientFactory`] and constructs a [`TestController`].
    pub fn from_config(config: BenchmarkConfig) -> Result<Self> {
        let client = HttpClientFactory::build(&config)?;
        Ok(Self::new(Arc::new(config), client))
    }

    /// Overrides the [`CancellationToken`] used to coordinate benchmark shutdown.
    pub fn with_cancel_token(mut self, cancel_token: CancellationToken) -> Self {
        self.cancel_token = cancel_token;
        self
    }

    /// Overrides the shared [`LiveCounters`] instance used for real-time telemetry.
    pub fn with_counters(mut self, counters: Arc<LiveCounters>) -> Self {
        self.counters = counters;
        self
    }

    /// Returns a clone of the shared [`LiveCounters`] updated in real-time during the run.
    pub fn counters(&self) -> Arc<LiveCounters> {
        Arc::clone(&self.counters)
    }

    /// Returns a clone of the controller's [`CancellationToken`].
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    /// Returns a reference to the shared [`BenchmarkConfig`].
    pub fn config(&self) -> &Arc<BenchmarkConfig> {
        &self.config
    }

    /// Returns a reference to the underlying [`reqwest::Client`].
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Executes the full load test lifecycle and returns the aggregated [`BenchmarkStats`].
    pub async fn run(&self) -> Result<BenchmarkStats> {
        let (stats, _) = self.run_with_spawn_timestamps().await?;
        Ok(stats)
    }

    /// Executes the full load test lifecycle and returns both the aggregated [`BenchmarkStats`]
    /// and the exact [`Instant`] at which each worker task was spawned.
    pub async fn run_with_spawn_timestamps(&self) -> Result<(BenchmarkStats, Vec<Instant>)> {
        let (tx, rx) = mpsc::unbounded_channel::<MetricEvent>();
        let start_time = Instant::now();
        let cancel_token = self.cancel_token.clone();

        // 1. Spawn the metrics collector task.
        let collector_handle = spawn_collector(rx, Arc::clone(&self.counters), start_time);

        // 2. Spawn the worker pool (immediately if ramp_up == 0, or concurrently across ramp_up).
        let (mut immediate_handles, mut immediate_timestamps, spawner_handle) =
            if self.config.ramp_up.is_zero() {
                let (handles, timestamps) = spawner::spawn_workers_with_timestamps(
                    Arc::clone(&self.config),
                    self.client.clone(),
                    tx,
                    cancel_token.clone(),
                )
                .await;
                (handles, timestamps, None)
            } else {
                let spawner_config = Arc::clone(&self.config);
                let spawner_client = self.client.clone();
                let spawner_token = cancel_token.clone();
                let handle = tokio::spawn(async move {
                    spawner::spawn_workers_with_timestamps(
                        spawner_config,
                        spawner_client,
                        tx,
                        spawner_token,
                    )
                    .await
                });
                (Vec::new(), Vec::new(), Some(handle))
            };

        // 3. Race benchmark duration against Ctrl+C and external cancellation.
        tokio::select! {
            _ = tokio::time::sleep(self.config.duration) => {}
            _ = signals::wait_for_shutdown_signal(cancel_token.clone()) => {}
            _ = cancel_token.cancelled() => {}
        }

        // 4. Signal all workers (and any in-progress ramp-up spawner) to stop.
        cancel_token.cancel();

        let (mut worker_handles, spawn_timestamps) = match spawner_handle {
            Some(handle) => handle
                .await
                .context("worker spawner task panicked during shutdown")?,
            None => {
                let handles = std::mem::take(&mut immediate_handles);
                let timestamps = std::mem::take(&mut immediate_timestamps);
                (handles, timestamps)
            }
        };

        // 5. Await all worker tasks with a 3-second grace timeout.
        let join_workers = async {
            for handle in &mut worker_handles {
                let _ = handle.await;
            }
        };

        if tokio::time::timeout(SHUTDOWN_GRACE_PERIOD, join_workers)
            .await
            .is_err()
        {
            for handle in &worker_handles {
                handle.abort();
            }
            for handle in worker_handles {
                let _ = handle.await;
            }
        }

        // 6. All worker senders and the spawner sender have now been dropped; await collector EOF.
        let stats = collector_handle
            .await
            .context("metrics collector task panicked")?;

        Ok((stats, spawn_timestamps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderMap;
    use reqwest::{Method, Url};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(url: &str, concurrency: usize, duration: Duration, ramp_up: Duration) -> BenchmarkConfig {
        BenchmarkConfig {
            url: Url::parse(url).expect("valid url"),
            concurrency,
            duration,
            method: Method::GET,
            headers: HeaderMap::new(),
            body: None,
            timeout: Duration::from_secs(2),
            ramp_up,
            output_json: None,
        }
    }

    #[tokio::test]
    async fn test_controller_runs_and_collects_stats() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ping"))
            .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
            .mount(&server)
            .await;

        let url = format!("{}/ping", server.uri());
        let config = test_config(&url, 4, Duration::from_millis(200), Duration::ZERO);
        let controller = TestController::from_config(config).expect("valid controller");
        let counters = controller.counters();

        let stats = controller.run().await.expect("benchmark run should succeed");

        assert!(stats.total_requests > 0, "expected requests to be recorded");
        assert_eq!(stats.total_requests, stats.successful_requests);
        assert_eq!(counters.total_requests(), stats.total_requests);
    }

    #[tokio::test]
    async fn test_controller_early_cancellation_during_ramp_up() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ramp"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let url = format!("{}/ramp", server.uri());
        let config = test_config(
            &url,
            10,
            Duration::from_secs(5),
            Duration::from_secs(2),
        );
        let cancel_token = CancellationToken::new();
        let controller = TestController::from_config(config)
            .expect("valid controller")
            .with_cancel_token(cancel_token.clone());

        let cancel_trigger = cancel_token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            cancel_trigger.cancel();
        });

        let start = Instant::now();
        let (stats, timestamps) = controller
            .run_with_spawn_timestamps()
            .await
            .expect("cancelled run should still return partial stats");
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "controller should stop promptly on cancellation, took {elapsed:?}"
        );
        assert!(
            !timestamps.is_empty() && timestamps.len() < 10,
            "expected partial worker spawn count when cancelled mid-ramp-up, got {}",
            timestamps.len()
        );
        assert!(stats.total_requests > 0);
    }
}
