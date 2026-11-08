//! Live terminal progress bar powered by `indicatif` polling atomic [`LiveCounters`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::metrics::stats::LiveCounters;

/// Polling interval for refreshing the live terminal progress bar.
pub const LIVE_TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Progress bar template displaying elapsed time, visual bar, position/duration, and live metrics.
pub const PROGRESS_TEMPLATE: &str =
    "[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s | {msg}";

/// Computes instantaneous requests per second over `delta_t_secs`.
pub fn compute_instantaneous_rps(requests_prev: u64, requests_now: u64, delta_t_secs: f64) -> f64 {
    if delta_t_secs <= 0.0 {
        return 0.0;
    }
    let delta_reqs = requests_now.saturating_sub(requests_prev);
    delta_reqs as f64 / delta_t_secs
}

/// Formats the dynamic telemetry message segment of the progress bar.
pub fn format_live_message(total_requests: u64, rps: f64, total_errors: u64) -> String {
    format!("Req: {total_requests} | RPS: {rps:.1} | Err: {total_errors}")
}

/// Creates a configured [`ProgressBar`] for the given total benchmark `duration`.
pub fn create_progress_bar(duration: Duration) -> ProgressBar {
    let total_secs = duration.as_secs().max(1);
    let pb = ProgressBar::new(total_secs);
    let style = ProgressStyle::with_template(PROGRESS_TEMPLATE)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-");
    pb.set_style(style);
    pb.set_message(format_live_message(0, 0.0, 0));
    pb
}

/// Spawns a background Tokio task that updates an `indicatif::ProgressBar` every 100ms
/// by atomically reading [`LiveCounters`] until `cancel_token` is cancelled.
///
/// Upon completion or cancellation, `finish_and_clear()` is invoked so subsequent
/// terminal summary tables are not clobbered.
pub fn spawn_live_progress_bar(
    duration: Duration,
    counters: Arc<LiveCounters>,
    cancel_token: CancellationToken,
) -> JoinHandle<()> {
    let pb = create_progress_bar(duration);
    let total_secs = duration.as_secs();

    tokio::spawn(async move {
        let start = Instant::now();
        let mut last_tick = start;
        let mut requests_prev = counters.total_requests();
        let mut ticker = tokio::time::interval(LIVE_TICK_INTERVAL);
        // First tick completes immediately; consume it so the loop waits 100ms.
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    break;
                }
                _ = ticker.tick() => {
                    let now = Instant::now();
                    let delta_t = now.duration_since(last_tick).as_secs_f64();
                    last_tick = now;

                    let requests_now = counters.total_requests();
                    let errors_now = counters.total_errors();
                    let rps = compute_instantaneous_rps(requests_prev, requests_now, delta_t);
                    requests_prev = requests_now;

                    let elapsed_secs = now.duration_since(start).as_secs();
                    let pos = if total_secs > 0 {
                        elapsed_secs.min(total_secs)
                    } else {
                        elapsed_secs
                    };

                    pb.set_position(pos);
                    pb.set_message(format_live_message(requests_now, rps, errors_now));
                }
            }
        }

        pb.finish_and_clear();
    })
}
