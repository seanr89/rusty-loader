use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Method, StatusCode, Url};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rusty_loader::client::{build_client, HttpClientFactory, DEFAULT_USER_AGENT};
use rusty_loader::config::BenchmarkConfig;
use rusty_loader::engine::worker::Worker;
use rusty_loader::os::{check_os_limits, OsLimits, RECOMMENDED_MIN_NOFILE};

fn make_config(url: &str, concurrency: usize, timeout: Duration) -> BenchmarkConfig {
    BenchmarkConfig {
        url: Url::parse(url).expect("valid test url"),
        concurrency,
        duration: Duration::from_secs(5),
        method: Method::GET,
        headers: HeaderMap::new(),
        body: None,
        timeout,
        ramp_up: Duration::ZERO,
        output_json: None,
    }
}

#[tokio::test]
async fn test_connection_pooling_and_client_policy() {
    let mock_server = MockServer::start().await;

    // Redirect endpoint to verify Redirect::Policy::none() is enforced
    Mock::given(method("GET"))
        .and(path("/redirect"))
        .and(header("user-agent", DEFAULT_USER_AGENT))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/target", mock_server.uri())),
        )
        .mount(&mock_server)
        .await;

    let config = make_config(
        &format!("{}/redirect", mock_server.uri()),
        700,
        Duration::from_secs(5),
    );
    let client = HttpClientFactory::build(&config).expect("client build should succeed");

    let (tx, _rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let worker = Worker::new(0, client, Arc::new(config), tx, cancel);

    let event = worker.execute_once().await;
    // Because redirect policy is none(), worker records the 302 directly without following
    assert_eq!(event.status_code, Some(StatusCode::FOUND));
    assert_eq!(event.error, None);
}

#[tokio::test]
async fn test_worker_single_iteration_50_byte_body_and_custom_request() {
    let mock_server = MockServer::start().await;
    let payload_50_bytes = "01234567890123456789012345678901234567890123456789";
    assert_eq!(payload_50_bytes.len(), 50);

    Mock::given(method("POST"))
        .and(path("/api/items"))
        .and(header("x-custom-header", "benchmark-value"))
        .and(header("content-type", "application/json"))
        .and(body_string(r#"{"ping":"pong"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_string(payload_50_bytes))
        .expect(1..)
        .mount(&mock_server)
        .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-custom-header"),
        HeaderValue::from_static("benchmark-value"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let mut config = make_config(
        &format!("{}/api/items", mock_server.uri()),
        10,
        Duration::from_secs(5),
    );
    config.method = Method::POST;
    config.headers = headers;
    config.body = Some(r#"{"ping":"pong"}"#.to_string());

    let client = build_client(&config).expect("client build should succeed");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();

    let worker = Worker::new(1, client, Arc::new(config), tx, cancel.clone());
    assert_eq!(worker.id(), 1);

    // Verify single iteration directly
    let single_event = worker.execute_once().await;
    assert_eq!(single_event.status_code, Some(StatusCode::OK));
    assert_eq!(single_event.bytes_received, 50);
    assert!(
        single_event.latency > Duration::ZERO,
        "expected latency > 0, got {:?}",
        single_event.latency
    );
    assert_eq!(single_event.error, None);

    // Also verify running in background loop and receiving at least one event via channel
    let handle = tokio::spawn(worker.run());
    let streamed_event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for worker event")
        .expect("channel closed unexpectedly");

    cancel.cancel();
    handle.await.expect("worker task panicked");

    assert_eq!(streamed_event.status_code, Some(StatusCode::OK));
    assert_eq!(streamed_event.bytes_received, 50);
    assert!(streamed_event.latency > Duration::ZERO);
    assert_eq!(streamed_event.error, None);
}

#[tokio::test]
async fn test_worker_cancellation_terminates_promptly() {
    let mock_server = MockServer::start().await;

    // Endpoint delays for 5 seconds, but we will cancel after 50ms
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("slow response")
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&mock_server)
        .await;

    let config = make_config(
        &format!("{}/slow", mock_server.uri()),
        4,
        Duration::from_secs(10),
    );
    let client = build_client(&config).expect("client build should succeed");
    let (tx, _rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();

    let worker = Worker::new(2, client, Arc::new(config), tx, cancel.clone());
    let handle = tokio::spawn(worker.run());

    tokio::time::sleep(Duration::from_millis(50)).await;
    let cancel_start = Instant::now();
    cancel.cancel();

    tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .expect("worker did not terminate promptly after cancellation")
        .expect("worker task panicked");

    assert!(
        cancel_start.elapsed() < Duration::from_millis(500),
        "worker took too long to stop after cancellation"
    );
}

#[tokio::test]
async fn test_worker_timeout_handling_emits_error_event_without_panic() {
    let mock_server = MockServer::start().await;

    // Endpoint delays for 300ms while client timeout is 50ms
    Mock::given(method("GET"))
        .and(path("/timeout"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("too late")
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&mock_server)
        .await;

    let config = make_config(
        &format!("{}/timeout", mock_server.uri()),
        4,
        Duration::from_millis(50),
    );
    let client = build_client(&config).expect("client build should succeed");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();

    let worker = Worker::new(3, client, Arc::new(config), tx, cancel.clone());
    let handle = tokio::spawn(worker.run());

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for timeout MetricEvent")
        .expect("worker channel closed without sending timeout event");

    cancel.cancel();
    handle.await.expect("worker panicked on timeout");

    assert_eq!(event.status_code, None);
    assert_eq!(event.bytes_received, 0);
    assert_eq!(event.error.as_deref(), Some("Timeout"));
    assert!(event.latency >= Duration::from_millis(40));
}

#[test]
fn test_os_limits_inspection_and_warning_formatting() {
    let limits = check_os_limits();
    assert_eq!(limits.recommended_min, RECOMMENDED_MIN_NOFILE);
    assert!(limits.soft_limit > 0);

    let simulated_low = OsLimits {
        soft_limit: 256,
        hard_limit: 65536,
        recommended_min: 4096,
    };
    assert!(!simulated_low.is_sufficient());
    assert_eq!(
        simulated_low.warning_message().as_deref(),
        Some("Warning: OS file descriptor limit is 256 (recommended >= 4096). Run 'ulimit -n 65536' before testing.")
    );

    let simulated_high = OsLimits {
        soft_limit: 65536,
        hard_limit: 65536,
        recommended_min: 4096,
    };
    assert!(simulated_high.is_sufficient());
    assert!(simulated_high.warning_message().is_none());
}
