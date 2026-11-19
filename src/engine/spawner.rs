//! Worker task spawner supporting immediate and staggered ramp-up schedules.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::BenchmarkConfig;
use crate::engine::worker::Worker;
use crate::metrics::event::MetricEvent;

/// Computes the delay between consecutive worker spawns for a given `ramp_up` duration
/// and `concurrency` level (`ramp_up / concurrency`).
pub fn compute_stagger_interval(ramp_up: Duration, concurrency: usize) -> Duration {
    if ramp_up.is_zero() || concurrency == 0 {
        Duration::ZERO
    } else {
        ramp_up / (concurrency as u32)
    }
}

/// Spawns `config.concurrency` worker tasks, either immediately (`ramp_up == Duration::ZERO`)
/// or staggered evenly across `config.ramp_up`.
///
/// If `cancel_token` is cancelled while ramping up, spawning halts immediately and returns
/// the handles of the workers launched so far.
pub async fn spawn_workers(
    config: Arc<BenchmarkConfig>,
    client: Client,
    sender: UnboundedSender<MetricEvent>,
    cancel_token: CancellationToken,
) -> Vec<JoinHandle<()>> {
    let (handles, _) =
        spawn_workers_with_timestamps(config, client, sender, cancel_token).await;
    handles
}

/// Spawns `config.concurrency` worker tasks and records the exact [`Instant`] each worker was spawned.
///
/// Useful for verifying ramp-up scheduling precision in tests alongside returning the worker [`JoinHandle`]s.
pub async fn spawn_workers_with_timestamps(
    config: Arc<BenchmarkConfig>,
    client: Client,
    sender: UnboundedSender<MetricEvent>,
    cancel_token: CancellationToken,
) -> (Vec<JoinHandle<()>>, Vec<Instant>) {
    let concurrency = config.concurrency;
    let stagger_interval = compute_stagger_interval(config.ramp_up, concurrency);

    let mut handles = Vec::with_capacity(concurrency);
    let mut timestamps = Vec::with_capacity(concurrency);

    for id in 0..concurrency {
        if cancel_token.is_cancelled() {
            break;
        }

        if id > 0 && !stagger_interval.is_zero() {
            tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    break;
                }
                _ = tokio::time::sleep(stagger_interval) => {}
            }

            if cancel_token.is_cancelled() {
                break;
            }
        }

        let spawned_at = Instant::now();
        let worker = Worker::new(
            id,
            client.clone(),
            Arc::clone(&config),
            sender.clone(),
            cancel_token.clone(),
        );
        let handle = tokio::spawn(worker.run());

        timestamps.push(spawned_at);
        handles.push(handle);
    }

    (handles, timestamps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_stagger_interval() {
        assert_eq!(
            compute_stagger_interval(Duration::ZERO, 50),
            Duration::ZERO
        );
        assert_eq!(
            compute_stagger_interval(Duration::from_millis(500), 10),
            Duration::from_millis(50)
        );
        assert_eq!(
            compute_stagger_interval(Duration::from_secs(1), 0),
            Duration::ZERO
        );
    }
}
