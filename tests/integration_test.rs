use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use rusty_loader::config::BenchmarkConfig;
use rusty_loader::engine::TestController;
use rusty_loader::reporter::JsonReport;
use serde_json::Value;
use tempfile::tempdir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request as WiremockRequest, Respond, ResponseTemplate};

/// Custom wiremock responder that deterministically returns:
/// - 70% `200 OK` (indices 0..7)
/// - 20% `429 Too Many Requests` (indices 7..9)
/// - 10% `500 Internal Server Error` (index 9)
struct MixedStatusResponder {
    counter: AtomicUsize,
}

impl MixedStatusResponder {
    fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }
}

impl Respond for MixedStatusResponder {
    fn respond(&self, _request: &WiremockRequest) -> ResponseTemplate {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % 10;
        match idx {
            0..=6 => ResponseTemplate::new(200).set_body_string(r#"{"status":"ok"}"#),
            7..=8 => ResponseTemplate::new(429).set_body_string(r#"{"error":"rate_limited"}"#),
            _ => ResponseTemplate::new(500).set_body_string(r#"{"error":"internal_error"}"#),
        }
    }
}

fn build_config(
    url: &str,
    concurrency: usize,
    duration: Duration,
    timeout: Duration,
) -> BenchmarkConfig {
    BenchmarkConfig {
        url: Url::parse(url).expect("valid URL"),
        concurrency,
        duration,
        method: Method::GET,
        headers: HeaderMap::new(),
        body: None,
        timeout,
        ramp_up: Duration::ZERO,
        output_json: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_happy_path_load_test() {
    let server = MockServer::start().await;
    let body_payload = r#"{"status":"ok","service":"rusty-loader","version":"0.1.0"}"#;

    Mock::given(method("GET"))
        .and(path("/v1/ping"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/json")
                .set_body_string(body_payload),
        )
        .mount(&server)
        .await;

    let target_url = format!("{}/v1/ping", server.uri());
    let config = build_config(
        &target_url,
        50,
        Duration::from_secs(2),
        Duration::from_secs(5),
    );

    let controller = TestController::from_config(config).expect("failed to build TestController");
    let stats = controller
        .run()
        .await
        .expect("happy path load test should succeed");

    assert!(
        stats.total_requests > 100,
        "expected > 100 total requests at 50 concurrency over 2s, got {}",
        stats.total_requests
    );
    assert_eq!(
        stats.successful_requests, stats.total_requests,
        "all requests should succeed with 200 OK"
    );
    assert_eq!(stats.client_errors, 0);
    assert_eq!(stats.server_errors, 0);
    assert_eq!(stats.transport_errors, 0);
    assert_eq!(
        stats.status_distribution.get(&200).copied().unwrap_or(0),
        stats.total_requests
    );
    assert!(stats.total_bytes > 0);
    assert!(stats.requests_per_sec > 0.0);

    // Verify HDR histogram percentile ordering invariants:
    // 0 < min <= p50 <= p90 <= p95 <= p99 <= p99.9 <= max
    let lat = &stats.latencies;
    assert!(
        lat.min_ms > 0.0,
        "expected min_ms > 0.0, got {}",
        lat.min_ms
    );
    assert!(
        lat.min_ms <= lat.p50_ms,
        "expected min_ms ({}) <= p50_ms ({})",
        lat.min_ms,
        lat.p50_ms
    );
    assert!(
        lat.p50_ms <= lat.p90_ms,
        "expected p50_ms ({}) <= p90_ms ({})",
        lat.p50_ms,
        lat.p90_ms
    );
    assert!(
        lat.p90_ms <= lat.p95_ms,
        "expected p90_ms ({}) <= p95_ms ({})",
        lat.p90_ms,
        lat.p95_ms
    );
    assert!(
        lat.p95_ms <= lat.p99_ms,
        "expected p95_ms ({}) <= p99_ms ({})",
        lat.p95_ms,
        lat.p99_ms
    );
    assert!(
        lat.p99_ms <= lat.p99_9_ms,
        "expected p99_ms ({}) <= p99_9_ms ({})",
        lat.p99_ms,
        lat.p99_9_ms
    );
    assert!(
        lat.p99_9_ms <= lat.max_ms,
        "expected p99_9_ms ({}) <= max_ms ({})",
        lat.p99_9_ms,
        lat.max_ms
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_mixed_http_status_codes() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/mixed"))
        .respond_with(MixedStatusResponder::new())
        .mount(&server)
        .await;

    let target_url = format!("{}/v1/mixed", server.uri());
    let config = build_config(
        &target_url,
        20,
        Duration::from_millis(800),
        Duration::from_secs(5),
    );

    let controller = TestController::from_config(config).expect("failed to build TestController");
    let stats = controller
        .run()
        .await
        .expect("mixed status load test should succeed");

    assert!(
        stats.total_requests >= 30,
        "expected at least 30 requests to sample all status buckets, got {}",
        stats.total_requests
    );

    let count_200 = stats.status_distribution.get(&200).copied().unwrap_or(0);
    let count_429 = stats.status_distribution.get(&429).copied().unwrap_or(0);
    let count_500 = stats.status_distribution.get(&500).copied().unwrap_or(0);

    assert!(count_200 > 0, "expected 200 OK responses to be recorded");
    assert!(
        count_429 > 0,
        "expected 429 Too Many Requests responses to be recorded"
    );
    assert!(
        count_500 > 0,
        "expected 500 Internal Server Error responses to be recorded"
    );

    assert_eq!(stats.successful_requests, count_200);
    assert_eq!(stats.client_errors, count_429);
    assert_eq!(stats.server_errors, count_500);
    assert_eq!(stats.transport_errors, 0);
    assert_eq!(
        stats.successful_requests + stats.client_errors + stats.server_errors,
        stats.total_requests
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_client_timeout_handling() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_string("delayed response"),
        )
        .mount(&server)
        .await;

    let target_url = format!("{}/v1/slow", server.uri());

    // 1. Verify library-level controller timeout handling
    let config = build_config(
        &target_url,
        10,
        Duration::from_millis(450),
        Duration::from_millis(100),
    );
    let controller = TestController::from_config(config).expect("failed to build TestController");
    let stats = controller
        .run()
        .await
        .expect("controller should complete cleanly on request timeouts");

    assert!(
        stats.total_requests > 0,
        "expected timeout attempts to be recorded"
    );
    assert_eq!(
        stats.successful_requests, 0,
        "no 500ms delayed request should succeed with a 100ms timeout"
    );
    assert_eq!(
        stats.transport_errors, stats.total_requests,
        "all timed-out requests must be counted under transport_errors"
    );
    assert_eq!(
        stats.error_breakdown.get("Timeout").copied().unwrap_or(0),
        stats.total_requests,
        "all errors should be classified as Timeout"
    );

    // 2. Verify compiled binary exits cleanly with status 0 when requests time out
    let dir = tempdir().expect("create temp dir");
    let report_path = dir.path().join("timeout_report.json");

    let status = Command::new(env!("CARGO_BIN_EXE_rusty-loader"))
        .args([
            "--url",
            &target_url,
            "--concurrency",
            "8",
            "--duration",
            "400ms",
            "--timeout",
            "100ms",
            "--output-json",
            report_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute rusty-loader binary");

    assert!(
        status.status.success(),
        "rusty-loader binary should exit with code 0 on client timeouts, stderr: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let json_text = std::fs::read_to_string(&report_path).expect("read timeout JSON report");
    let report: JsonReport =
        serde_json::from_str(&json_text).expect("deserialize timeout JsonReport");
    assert!(report.stats.total_requests > 0);
    assert_eq!(report.stats.successful_requests, 0);
    assert_eq!(report.stats.transport_errors, report.stats.total_requests);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_json_output_verification_end_to_end() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/query"))
        .and(header("Authorization", "Bearer e2e-test-token"))
        .and(header("Content-Type", "application/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/json")
                .set_body_string(r#"{"records":[1,2,3],"count":3}"#),
        )
        .mount(&server)
        .await;

    let target_url = format!("{}/v1/query", server.uri());
    let dir = tempdir().expect("create temp dir");
    let report_path = dir.path().join("nested").join("e2e_report.json");

    let output = Command::new(env!("CARGO_BIN_EXE_api-load-tester"))
        .args([
            "--url",
            &target_url,
            "--method",
            "POST",
            "--header",
            "Authorization: Bearer e2e-test-token",
            "--header",
            "Content-Type: application/json",
            "--body",
            r#"{"query":"active"}"#,
            "--concurrency",
            "20",
            "--duration",
            "1s",
            "--timeout",
            "2000ms",
            "--ramp-up",
            "200ms",
            "--output-json",
            report_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute api-load-tester binary");

    assert!(
        output.status.success(),
        "api-load-tester exited with non-zero status: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        report_path.exists(),
        "expected JSON report file to be created at {}",
        report_path.display()
    );

    let raw_json = std::fs::read_to_string(&report_path).expect("read generated JSON file");
    let parsed_value: Value =
        serde_json::from_str(&raw_json).expect("generated JSON report must be valid JSON");

    // Verify required top-level keys exist
    for expected_key in [
        "timestamp",
        "url",
        "method",
        "concurrency",
        "configured_duration_secs",
        "headers",
        "total_duration_secs",
        "total_requests",
        "successful_requests",
        "client_errors",
        "server_errors",
        "transport_errors",
        "requests_per_sec",
        "total_bytes",
        "bytes_per_sec",
        "latencies",
        "status_distribution",
        "error_breakdown",
    ] {
        assert!(
            parsed_value.get(expected_key).is_some(),
            "missing expected key '{expected_key}' in JSON report: {raw_json}"
        );
    }

    // Verify nested latency keys exist
    let latencies_obj = parsed_value
        .get("latencies")
        .and_then(Value::as_object)
        .expect("latencies must be a JSON object");
    for latency_key in [
        "min_ms",
        "mean_ms",
        "p50_ms",
        "p90_ms",
        "p95_ms",
        "p99_ms",
        "p99_9_ms",
        "max_ms",
        "std_dev_ms",
    ] {
        assert!(
            latencies_obj.contains_key(latency_key),
            "missing latency key '{latency_key}' in JSON report"
        );
    }

    // Also verify typed deserialization into JsonReport
    let typed_report: JsonReport =
        serde_json::from_str(&raw_json).expect("deserialize into JsonReport");
    assert_eq!(typed_report.url, target_url);
    assert_eq!(typed_report.method, "POST");
    assert_eq!(typed_report.concurrency, 20);
    assert!(typed_report.stats.total_requests > 0);
    assert_eq!(
        typed_report.stats.successful_requests,
        typed_report.stats.total_requests
    );
    assert_eq!(
        typed_report
            .stats
            .status_distribution
            .get(&200)
            .copied()
            .unwrap_or(0),
        typed_report.stats.total_requests
    );
}
