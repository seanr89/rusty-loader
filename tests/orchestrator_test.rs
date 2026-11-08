use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rusty_loader::client::HttpClientFactory;
use rusty_loader::config::BenchmarkConfig;
use rusty_loader::engine::spawner;
use rusty_loader::engine::TestController;
use rusty_loader::metrics::MetricEvent;

fn build_test_config(
    url: &str,
    concurrency: usize,
    duration: Duration,
    ramp_up: Duration,
) -> BenchmarkConfig {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_immediate_execution_50_workers_1_second() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/immediate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("ok")
                .set_delay(Duration::from_millis(5)),
        )
        .mount(&server)
        .await;

    let target_url = format!("{}/immediate", server.uri());
    let config = build_test_config(&target_url, 50, Duration::from_secs(1), Duration::ZERO);
    let controller = TestController::from_config(config).expect("failed to build TestController");

    let run_start = Instant::now();
    let (stats, timestamps) = controller
        .run_with_spawn_timestamps()
        .await
        .expect("immediate execution benchmark failed");
    let total_elapsed = run_start.elapsed();

    assert_eq!(
        timestamps.len(),
        50,
        "all 50 workers must be spawned immediately"
    );

    let first_spawn = timestamps.first().copied().unwrap();
    let last_spawn = timestamps.last().copied().unwrap();
    let spawn_spread = last_spawn.saturating_duration_since(first_spawn);
    assert!(
        spawn_spread < Duration::from_millis(100),
        "all 50 workers should spawn almost simultaneously without ramp-up (spread was {spawn_spread:?})"
    );

    assert!(
        total_elapsed >= Duration::from_millis(950) && total_elapsed < Duration::from_millis(2500),
        "benchmark should complete cleanly near the 1s mark (took {total_elapsed:?})"
    );
    assert!(
        stats.total_requests >= 50,
        "expected at least 50 requests from 50 workers over 1s, got {}",
        stats.total_requests
    );
    assert_eq!(stats.successful_requests, stats.total_requests);
    assert_eq!(stats.status_distribution.get(&200).copied(), Some(stats.total_requests));
    assert!(stats.requests_per_sec > 0.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_ramp_up_timing_10_workers_500ms() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ramp-up"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("ramped")
                .set_delay(Duration::from_millis(10)),
        )
        .mount(&server)
        .await;

    let target_url = format!("{}/ramp-up", server.uri());
    let config = build_test_config(
        &target_url,
        10,
        Duration::from_millis(800),
        Duration::from_millis(500),
    );
    let controller = TestController::from_config(config).expect("failed to build TestController");

    let (stats, timestamps) = controller
        .run_with_spawn_timestamps()
        .await
        .expect("ramp-up benchmark failed");

    assert_eq!(timestamps.len(), 10, "all 10 workers should be spawned");
    assert!(stats.total_requests > 0, "requests should be recorded");

    // Verify each consecutive pair of workers is spaced approximately 50ms apart
    for pair in timestamps.windows(2) {
        let delta = pair[1].saturating_duration_since(pair[0]);
        assert!(
            delta >= Duration::from_millis(30) && delta <= Duration::from_millis(90),
            "expected ~50ms stagger interval between worker spawns, got {delta:?}"
        );
    }

    // Total span from worker 0 (at ~0ms) to worker 9 (at ~450ms) should be around 450ms
    let total_ramp_span = timestamps[9].saturating_duration_since(timestamps[0]);
    assert!(
        total_ramp_span >= Duration::from_millis(380)
            && total_ramp_span <= Duration::from_millis(620),
        "expected total 10-worker span across 9 intervals to be ~450ms, got {total_ramp_span:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_graceful_cancellation_returns_partial_stats_promptly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cancel"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("partial-payload")
                .set_delay(Duration::from_millis(5)),
        )
        .mount(&server)
        .await;

    let target_url = format!("{}/cancel", server.uri());
    // Configure a long 10-second duration, but trigger cancellation after 150ms.
    let config = build_test_config(&target_url, 20, Duration::from_secs(10), Duration::ZERO);
    let cancel_token = CancellationToken::new();
    let controller = TestController::from_config(config)
        .expect("failed to build TestController")
        .with_cancel_token(cancel_token.clone());

    let cancel_trigger = cancel_token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel_trigger.cancel();
    });

    let start = Instant::now();
    let stats = controller
        .run()
        .await
        .expect("cancelled run should cleanly return partial BenchmarkStats");
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "graceful shutdown should complete well before the 10s duration (took {elapsed:?})"
    );
    assert!(
        stats.total_requests > 0,
        "partial statistics should include completed requests before cancellation"
    );
    assert_eq!(stats.successful_requests, stats.total_requests);
    assert_eq!(
        stats.total_bytes,
        stats.total_requests * ("partial-payload".len() as u64)
    );
    assert!(stats.latencies.max_ms > 0.0);
}

#[tokio::test]
async fn test_spawner_stops_immediately_if_cancelled_mid_ramp_up() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/spawner-cancel"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let target_url = format!("{}/spawner-cancel", server.uri());
    let config = Arc::new(build_test_config(
        &target_url,
        10,
        Duration::from_secs(5),
        Duration::from_millis(500),
    ));
    let client = HttpClientFactory::build(&config).expect("valid client");
    let (tx, _rx) = mpsc::unbounded_channel::<MetricEvent>();
    let cancel_token = CancellationToken::new();

    let token_clone = cancel_token.clone();
    tokio::spawn(async move {
        // Cancel after ~125ms (should allow worker 0 at 0ms, worker 1 at 50ms, worker 2 at 100ms)
        tokio::time::sleep(Duration::from_millis(125)).await;
        token_clone.cancel();
    });

    let (handles, timestamps) =
        spawner::spawn_workers_with_timestamps(config, client, tx, cancel_token).await;

    assert!(
        timestamps.len() >= 2 && timestamps.len() <= 4,
        "expected ~3 workers spawned before 125ms cancellation, got {}",
        timestamps.len()
    );

    for handle in handles {
        handle.await.expect("worker task should exit cleanly");
    }
}
