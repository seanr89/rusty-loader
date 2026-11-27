#![cfg(feature = "gui")]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use rusty_loader::changelog::{AboutPayload, ChangelogPayload, APP_NAME, APP_VERSION};
use rusty_loader::config::{BenchmarkConfig, GuiConfig};
use rusty_loader::engine::TestController;
use rusty_loader::gui::{start_gui_server, MetaResponse};
use rusty_loader::metrics::{BenchmarkStats, LiveCounters, TelemetryHub, TelemetrySnapshot};
use rusty_loader::reporter::JsonReport;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn local_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("local reqwest client should build")
}

fn ephemeral_gui_config() -> GuiConfig {
    GuiConfig {
        enabled: true,
        port: 0,
        bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
        open_browser: false,
        keep_alive: false,
    }
}

fn assert_stats_match(actual: &BenchmarkStats, expected: &BenchmarkStats) {
    assert_eq!(actual.total_requests, expected.total_requests);
    assert_eq!(actual.successful_requests, expected.successful_requests);
    assert_eq!(actual.client_errors, expected.client_errors);
    assert_eq!(actual.server_errors, expected.server_errors);
    assert_eq!(actual.other_statuses, expected.other_statuses);
    assert_eq!(actual.transport_errors, expected.transport_errors);
    assert_eq!(actual.total_bytes, expected.total_bytes);
    assert_eq!(actual.status_distribution, expected.status_distribution);
    assert_eq!(actual.error_breakdown, expected.error_breakdown);
    assert!((actual.total_duration_secs - expected.total_duration_secs).abs() < 1e-9);
    assert!((actual.requests_per_sec - expected.requests_per_sec).abs() < 1e-6);
    assert!((actual.bytes_per_sec - expected.bytes_per_sec).abs() < 1e-6);
    assert!((actual.latencies.min_ms - expected.latencies.min_ms).abs() < 1e-9);
    assert!((actual.latencies.mean_ms - expected.latencies.mean_ms).abs() < 1e-9);
    assert!((actual.latencies.p50_ms - expected.latencies.p50_ms).abs() < 1e-9);
    assert!((actual.latencies.p90_ms - expected.latencies.p90_ms).abs() < 1e-9);
    assert!((actual.latencies.p95_ms - expected.latencies.p95_ms).abs() < 1e-9);
    assert!((actual.latencies.p99_ms - expected.latencies.p99_ms).abs() < 1e-9);
    assert!((actual.latencies.p99_9_ms - expected.latencies.p99_9_ms).abs() < 1e-9);
    assert!((actual.latencies.max_ms - expected.latencies.max_ms).abs() < 1e-9);
    assert!((actual.latencies.std_dev_ms - expected.latencies.std_dev_ms).abs() < 1e-9);
}

async fn read_sse_events(
    mut response: reqwest::Response,
    timeout_dur: Duration,
) -> Vec<(String, String)> {
    let mut buffer = String::new();
    let mut events = Vec::new();

    let read_fut = async {
        while let Ok(Some(chunk)) = response.chunk().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(sep_idx) = buffer.find("\n\n") {
                let raw_frame = buffer[..sep_idx].to_string();
                buffer.drain(..sep_idx + 2);

                let mut event_name = String::from("message");
                let mut data_lines = Vec::new();
                for line in raw_frame.lines() {
                    let line = line.trim_end_matches('\r');
                    if line.starts_with(':') || line.is_empty() {
                        continue;
                    }
                    if let Some(val) = line.strip_prefix("event:") {
                        event_name = val.trim().to_string();
                    } else if let Some(val) = line.strip_prefix("data:") {
                        data_lines.push(val.trim_start().to_string());
                    }
                }

                if !data_lines.is_empty() {
                    let payload = data_lines.join("\n");
                    let is_complete = event_name == "complete";
                    events.push((event_name, payload));
                    if is_complete {
                        return events;
                    }
                }
            }
        }
        events
    };

    tokio::time::timeout(timeout_dur, read_fut)
        .await
        .expect("timed out reading SSE events")
}

#[tokio::test]
async fn test_gui_static_assets_and_meta_header_redaction() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer top-secret-jwt"),
    );
    headers.insert(
        HeaderName::from_static("cookie"),
        HeaderValue::from_static("session_id=secret123"),
    );
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_static("ak_live_987654"),
    );
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        HeaderName::from_static("x-trace-id"),
        HeaderValue::from_static("trace-42"),
    );

    let config = Arc::new(BenchmarkConfig {
        url: Url::parse("https://svc_user:secretpass@api.example.com/v1/query").unwrap(),
        concurrency: 25,
        duration: Duration::from_secs(30),
        method: Method::POST,
        headers,
        body: Some(r#"{"ping":true}"#.to_string()),
        timeout: Duration::from_millis(2500),
        ramp_up: Duration::from_secs(5),
        output_json: None,
        output_csv: None,
        output_pdf: None,
        scenario: None,
    });

    let hub = Arc::new(TelemetryHub::new(config.duration, config.ramp_up));
    let counters = Arc::new(LiveCounters::new());
    let cancel_token = CancellationToken::new();

    let server_handle = start_gui_server(
        Arc::clone(&config),
        &ephemeral_gui_config(),
        hub,
        counters,
        cancel_token,
    )
    .await
    .expect("GUI server should bind ephemeral port");

    let base_url = server_handle.url();
    let client = local_http_client();

    // 1. HTML index, CSS, and JS must all serve offline without external CDN links
    let html_resp = client.get(&base_url).send().await.expect("GET /");
    assert_eq!(html_resp.status(), StatusCode::OK);
    let html = html_resp.text().await.unwrap();
    assert!(html.contains("rusty-loader"));
    assert!(html.contains(r#"<meta name="color-scheme" content="light dark">"#));
    assert!(!html.contains(r#"href="data:,""#));
    assert!(html.contains(r#"href="data:image/svg+xml;base64,"#));
    assert!(html.contains(r#"id="rl-brand-mark""#));
    assert!(html.contains(r##"href="#rl-brand-mark""##));
    assert!(!html.contains("https://"));
    assert!(!html.contains("http://"));

    let css_resp = client
        .get(format!("{base_url}/assets/styles.css"))
        .send()
        .await
        .expect("GET /assets/styles.css");
    assert_eq!(css_resp.status(), StatusCode::OK);
    let css = css_resp.text().await.unwrap();
    assert!(css.contains("color-scheme: light dark;"));
    assert!(!css.contains("https://"));

    let js_resp = client
        .get(format!("{base_url}/assets/app.js"))
        .send()
        .await
        .expect("GET /assets/app.js");
    assert_eq!(js_resp.status(), StatusCode::OK);
    let js = js_resp.text().await.unwrap();
    assert!(js.contains("/api/stream"));
    assert!(js.contains("/api/stop"));
    assert!(!js.contains("https://"));

    // 2. /api/meta must redact sensitive headers and URL userinfo
    let meta_resp = client
        .get(format!("{base_url}/api/meta"))
        .send()
        .await
        .expect("GET /api/meta");
    assert_eq!(meta_resp.status(), StatusCode::OK);
    let meta: MetaResponse = meta_resp.json().await.expect("valid MetaResponse JSON");

    assert!(!meta.url.contains("secretpass"));
    assert!(!meta.url.contains("svc_user"));
    assert!(meta.url.contains("api.example.com/v1/query"));
    assert_eq!(meta.method, "POST");
    assert_eq!(meta.concurrency, 25);
    assert!(meta.has_body);
    assert_eq!(meta.body_size_bytes, r#"{"ping":true}"#.len());
    assert_eq!(
        meta.headers.get("authorization").map(String::as_str),
        Some("[REDACTED]")
    );
    assert_eq!(
        meta.headers.get("cookie").map(String::as_str),
        Some("[REDACTED]")
    );
    assert_eq!(
        meta.headers.get("x-api-key").map(String::as_str),
        Some("[REDACTED]")
    );
    assert_eq!(
        meta.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        meta.headers.get("x-trace-id").map(String::as_str),
        Some("trace-42")
    );

    // 3. /api/about and /api/changelog must return embedded version & changelog offline
    let about_resp = client
        .get(format!("{base_url}/api/about"))
        .send()
        .await
        .expect("GET /api/about");
    assert_eq!(about_resp.status(), StatusCode::OK);
    let about: AboutPayload = about_resp.json().await.expect("valid AboutPayload JSON");
    assert_eq!(about.name, APP_NAME);
    assert_eq!(about.version, APP_VERSION);
    let latest = about
        .latest_change
        .expect("latest_change should be populated from embedded CHANGELOG.md");
    assert_eq!(latest.version, APP_VERSION);
    assert!(!latest.notes.is_empty());

    let changelog_resp = client
        .get(format!("{base_url}/api/changelog"))
        .send()
        .await
        .expect("GET /api/changelog");
    assert_eq!(changelog_resp.status(), StatusCode::OK);
    let changelog: ChangelogPayload = changelog_resp
        .json()
        .await
        .expect("valid ChangelogPayload JSON");
    assert_eq!(changelog.current_version, APP_VERSION);
    assert!(changelog.raw_markdown.contains("# Changelog"));
    assert!(!changelog.releases.is_empty());
    assert_eq!(changelog.releases[0].version, latest.version);

    // Verify About dialog markup exists in index.html
    assert!(html.contains(r#"id="about-dialog""#));
    assert!(html.contains(r#"id="about-btn""#));

    server_handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_gui_sse_streaming_history_hydration_and_json_export() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ping"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("pong")
                .set_delay(Duration::from_millis(5)),
        )
        .mount(&mock_server)
        .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer export-secret"),
    );

    let config = BenchmarkConfig {
        url: Url::parse(&format!("{}/ping", mock_server.uri())).unwrap(),
        concurrency: 6,
        duration: Duration::from_millis(450),
        method: Method::GET,
        headers,
        body: None,
        timeout: Duration::from_secs(2),
        ramp_up: Duration::ZERO,
        output_json: None,
        output_csv: None,
        output_pdf: None,
        scenario: None,
    };

    let hub = Arc::new(TelemetryHub::with_options(
        config.duration,
        config.ramp_up,
        Duration::from_millis(40),
        500,
        64,
    ));

    let controller = TestController::from_config(config)
        .expect("controller should build")
        .with_telemetry_hub(Arc::clone(&hub));

    let server_handle = start_gui_server(
        Arc::clone(controller.config()),
        &ephemeral_gui_config(),
        Arc::clone(&hub),
        controller.counters(),
        controller.cancel_token(),
    )
    .await
    .expect("GUI server should start");

    let base_url = server_handle.url();
    let client = local_http_client();

    let sse_response = client
        .get(format!("{base_url}/api/stream"))
        .send()
        .await
        .expect("connect to /api/stream");
    assert_eq!(sse_response.status(), StatusCode::OK);

    let sse_task = tokio::spawn(read_sse_events(sse_response, Duration::from_secs(5)));
    let final_stats = controller.run().await.expect("benchmark run should succeed");
    let events = sse_task.await.expect("SSE task should join cleanly");

    let snapshot_events: Vec<TelemetrySnapshot> = events
        .iter()
        .filter(|(name, _)| name == "snapshot")
        .map(|(_, data)| serde_json::from_str(data).expect("valid TelemetrySnapshot SSE JSON"))
        .collect();
    assert!(
        !snapshot_events.is_empty(),
        "expected at least one SSE snapshot event during benchmark run"
    );

    let complete_payload = events
        .iter()
        .find(|(name, _)| name == "complete")
        .map(|(_, data)| data)
        .expect("expected final SSE complete event");
    let streamed_stats: BenchmarkStats =
        serde_json::from_str(complete_payload).expect("valid BenchmarkStats in complete event");
    assert_stats_match(&streamed_stats, &final_stats);
    assert!(final_stats.total_requests > 0);

    // Verify /api/history hydrates snapshots chronologically from t=0
    let history: Vec<TelemetrySnapshot> = client
        .get(format!("{base_url}/api/history"))
        .send()
        .await
        .expect("GET /api/history")
        .json()
        .await
        .expect("valid history array");
    assert!(!history.is_empty());
    for pair in history.windows(2) {
        assert!(pair[0].elapsed_secs <= pair[1].elapsed_secs);
    }

    // Verify /api/export/json returns a valid JsonReport with redacted headers
    let export_resp = client
        .get(format!("{base_url}/api/export/json"))
        .send()
        .await
        .expect("GET /api/export/json");
    assert_eq!(export_resp.status(), StatusCode::OK);
    let export_report: JsonReport = export_resp
        .json()
        .await
        .expect("exported JSON should deserialize into JsonReport");
    assert_stats_match(&export_report.stats, &final_stats);
    assert_eq!(
        export_report.headers.get("authorization").map(String::as_str),
        Some("[REDACTED]")
    );

    // Verify /api/export/csv returns RFC 4180 CSV with redacted headers
    let csv_resp = client
        .get(format!("{base_url}/api/export/csv"))
        .send()
        .await
        .expect("GET /api/export/csv");
    assert_eq!(csv_resp.status(), StatusCode::OK);
    assert!(csv_resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("text/csv"));
    let csv_body = csv_resp.text().await.expect("CSV body text");
    assert!(!csv_body.contains("export-secret"));
    assert!(csv_body.contains("record_type,timestamp_ms,elapsed_secs"));
    assert!(csv_body.contains("\nsummary,") || csv_body.contains("\r\nsummary,"));

    // Verify /api/export/pdf returns valid PDF bytes with redacted headers
    let pdf_resp = client
        .get(format!("{base_url}/api/export/pdf"))
        .send()
        .await
        .expect("GET /api/export/pdf");
    assert_eq!(pdf_resp.status(), StatusCode::OK);
    assert_eq!(
        pdf_resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/pdf")
    );
    let pdf_bytes = pdf_resp.bytes().await.expect("PDF bytes");
    assert!(pdf_bytes.starts_with(b"%PDF-1.7"));
    let pdf_lossy = String::from_utf8_lossy(&pdf_bytes);
    assert!(pdf_lossy.trim_end().ends_with("%%EOF"));
    assert!(!pdf_lossy.contains("export-secret"));

    server_handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_gui_stop_endpoint_cancels_benchmark_gracefully() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/work"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("ok")
                .set_delay(Duration::from_millis(5)),
        )
        .mount(&mock_server)
        .await;

    let config = BenchmarkConfig {
        url: Url::parse(&format!("{}/work", mock_server.uri())).unwrap(),
        concurrency: 8,
        duration: Duration::from_secs(10),
        method: Method::GET,
        headers: HeaderMap::new(),
        body: None,
        timeout: Duration::from_secs(2),
        ramp_up: Duration::ZERO,
        output_json: None,
        output_csv: None,
        output_pdf: None,
        scenario: None,
    };

    let hub = Arc::new(TelemetryHub::with_options(
        config.duration,
        config.ramp_up,
        Duration::from_millis(40),
        500,
        64,
    ));

    let controller = Arc::new(
        TestController::from_config(config)
            .expect("controller should build")
            .with_telemetry_hub(Arc::clone(&hub)),
    );

    let server_handle = start_gui_server(
        Arc::clone(controller.config()),
        &ephemeral_gui_config(),
        Arc::clone(&hub),
        controller.counters(),
        controller.cancel_token(),
    )
    .await
    .expect("GUI server should start");

    let base_url = server_handle.url();
    let client = local_http_client();

    let sse_resp = client
        .get(format!("{base_url}/api/stream"))
        .send()
        .await
        .expect("connect to /api/stream");
    let sse_task = tokio::spawn(read_sse_events(sse_resp, Duration::from_secs(5)));

    let run_controller = Arc::clone(&controller);
    let started = Instant::now();
    let run_task = tokio::spawn(async move { run_controller.run().await });

    // Wait until workers have recorded at least a few requests
    let deadline = Instant::now() + Duration::from_secs(2);
    while controller.counters().total_requests() == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    assert!(
        controller.counters().total_requests() > 0,
        "expected benchmark to process requests before stopping"
    );

    let stop_resp = client
        .post(format!("{base_url}/api/stop"))
        .send()
        .await
        .expect("POST /api/stop");
    assert_eq!(stop_resp.status(), StatusCode::OK);
    let stop_body: serde_json::Value = stop_resp.json().await.unwrap();
    assert_eq!(stop_body["cancelled"], true);

    let stats = run_task
        .await
        .expect("controller task panicked")
        .expect("cancelled benchmark should return partial stats");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(3),
        "stopping via /api/stop should terminate promptly (took {elapsed:?})"
    );
    assert!(stats.total_requests > 0);

    let events = sse_task.await.expect("SSE task should complete");
    let complete_payload = events
        .iter()
        .find(|(name, _)| name == "complete")
        .map(|(_, data)| data)
        .expect("SSE stream should emit complete event on stop");
    let streamed_stats: BenchmarkStats = serde_json::from_str(complete_payload).unwrap();
    assert_stats_match(&streamed_stats, &stats);

    server_handle.shutdown().await;
}
