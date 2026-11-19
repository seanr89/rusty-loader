use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use colored::Color;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use rusty_loader::config::BenchmarkConfig;
use rusty_loader::metrics::stats::{BenchmarkStats, LatencyDistribution, LiveCounters};
use rusty_loader::reporter::json::{export_json_report, JsonReport};
use rusty_loader::reporter::live::{
    compute_instantaneous_rps, format_live_message, spawn_live_progress_bar,
};
use rusty_loader::reporter::table::{
    colorize_latency, format_bytes, format_latency, format_latency_colored, format_transfer_rate,
    render_summary_tables,
};

fn build_sample_config() -> BenchmarkConfig {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer token-123"),
    );

    BenchmarkConfig {
        url: Url::parse("https://api.example.com/v1/ping").expect("valid URL"),
        concurrency: 700,
        duration: Duration::from_secs(60),
        method: Method::POST,
        headers,
        body: Some(r#"{"ping":true}"#.to_string()),
        timeout: Duration::from_secs(5),
        ramp_up: Duration::from_secs(5),
        output_json: None,
    }
}

fn build_sample_stats(with_transport_errors: bool) -> BenchmarkStats {
    let mut status_distribution = HashMap::new();
    status_distribution.insert(200, 124_480);
    status_distribution.insert(502, 20);

    let mut error_breakdown = HashMap::new();
    let transport_errors = if with_transport_errors {
        error_breakdown.insert("RequestTimeout".to_string(), 12);
        error_breakdown.insert("ConnectError".to_string(), 3);
        15
    } else {
        0
    };

    BenchmarkStats {
        total_duration_secs: 60.0,
        total_requests: 124_500 + transport_errors,
        successful_requests: 124_480,
        client_errors: 0,
        server_errors: 20,
        other_statuses: 0,
        transport_errors,
        requests_per_sec: 2075.0,
        total_bytes: 54_945_382,
        bytes_per_sec: 915_756.36,
        latencies: LatencyDistribution {
            min_ms: 0.45,
            mean_ms: 4.21,
            p50_ms: 3.80,
            p90_ms: 7.10,
            p95_ms: 11.40,
            p99_ms: 18.90,
            p99_9_ms: 45.20,
            max_ms: 240.50,
            std_dev_ms: 3.15,
        },
        status_distribution,
        error_breakdown,
    }
}

#[test]
fn test_table_rendering_contains_expected_sections_and_metrics() {
    let config = build_sample_config();
    let stats = build_sample_stats(false);

    let output = render_summary_tables(&config, &stats);

    // Overview & throughput assertions
    assert!(output.contains("https://api.example.com/v1/ping"));
    assert!(output.contains("POST"));
    assert!(output.contains("700"));
    assert!(output.contains("60.00 s"));
    assert!(output.contains("124500"));
    assert!(output.contains("124480 (99.98%)"));
    assert!(output.contains("20 (0.02%)"));
    assert!(output.contains("2075.0 req/s"));
    assert!(output.contains("52.40 MB"));
    assert!(output.contains("894.29 KB/s"));

    // Latency percentiles assertions
    for label in [
        "Min",
        "Mean",
        "Median (p50)",
        "p90",
        "p95",
        "p99",
        "p99.9",
        "Max",
        "Std Dev",
    ] {
        assert!(
            output.contains(label),
            "expected latency table to contain '{label}'"
        );
    }
    assert!(output.contains("0.45 ms"));
    assert!(output.contains("3.80 ms"));
    assert!(output.contains("18.90 ms"));
    assert!(output.contains("45.20 ms"));
    assert!(output.contains("240.50 ms"));

    // HTTP status breakdown assertions
    assert!(output.contains("200"));
    assert!(output.contains("OK"));
    assert!(output.contains("502"));
    assert!(output.contains("Bad Gateway"));

    // Conditional error breakdown table should NOT appear when there are no transport errors
    assert!(!output.contains("=== Error Breakdown ==="));
}

#[test]
fn test_table_rendering_includes_conditional_error_breakdown() {
    let config = build_sample_config();
    let stats = build_sample_stats(true);

    let output = render_summary_tables(&config, &stats);

    assert!(output.contains("=== Error Breakdown ==="));
    assert!(output.contains("RequestTimeout"));
    assert!(output.contains("12"));
    assert!(output.contains("ConnectError"));
    assert!(output.contains("3"));
}

#[test]
fn test_json_output_file_creation_and_deserialization() {
    let dir = tempdir().expect("create temp dir");
    let nested_path = dir.path().join("nested").join("reports").join("bench.json");

    let config = build_sample_config();
    let stats = build_sample_stats(true);

    export_json_report(&nested_path, &config, &stats).expect("export json report");
    assert!(nested_path.exists(), "expected JSON file to be created");

    let raw_json = fs::read_to_string(&nested_path).expect("read exported JSON");

    // 1. Verify raw JSON deserializes directly into BenchmarkStats
    let deserialized_stats: BenchmarkStats =
        serde_json::from_str(&raw_json).expect("deserialize directly into BenchmarkStats");
    assert_eq!(deserialized_stats, stats);

    // 2. Verify raw JSON also deserializes into full JsonReport with execution metadata
    let deserialized_report: JsonReport =
        serde_json::from_str(&raw_json).expect("deserialize into JsonReport");
    assert_eq!(deserialized_report.url, "https://api.example.com/v1/ping");
    assert_eq!(deserialized_report.method, "POST");
    assert_eq!(deserialized_report.concurrency, 700);
    assert!((deserialized_report.configured_duration_secs - 60.0).abs() < 1e-6);
    assert_eq!(
        deserialized_report
            .headers
            .get("authorization")
            .map(String::as_str),
        Some("Bearer token-123")
    );
    assert!(!deserialized_report.timestamp.is_empty());
    assert_eq!(deserialized_report.stats, stats);

    // 3. Verify top-level JSON object keys exist as expected
    let json_val: serde_json::Value = serde_json::from_str(&raw_json).expect("valid json value");
    assert!(json_val.get("total_requests").is_some());
    assert!(json_val.get("latencies").is_some());
    assert!(json_val.get("status_distribution").is_some());
    assert!(json_val.get("timestamp").is_some());
    assert!(json_val.get("url").is_some());
}

#[test]
fn test_color_and_unit_formatting_helpers() {
    // Sub-50ms -> Green
    let fast = colorize_latency(1.24);
    assert_eq!(fast.fgcolor, Some(Color::Green));
    assert_eq!(&*fast, "1.24 ms");
    assert!(format_latency_colored(1.24).contains("1.24 ms"));

    // 50ms..=200ms -> Yellow
    let medium_low = colorize_latency(50.0);
    assert_eq!(medium_low.fgcolor, Some(Color::Yellow));
    assert_eq!(&*medium_low, "50.00 ms");

    let medium_high = colorize_latency(200.0);
    assert_eq!(medium_high.fgcolor, Some(Color::Yellow));
    assert_eq!(&*medium_high, "200.00 ms");

    // > 200ms -> Red
    let slow = colorize_latency(200.01);
    assert_eq!(slow.fgcolor, Some(Color::Red));
    assert_eq!(&*slow, "200.01 ms");

    assert_eq!(format_latency(0.456), "0.46 ms");

    // Byte & transfer rate helpers
    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(2048), "2.00 KB");
    assert_eq!(format_bytes(5 * 1024 * 1024), "5.00 MB");
    assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");

    assert_eq!(format_transfer_rate(500.0), "500.00 B/s");
    assert_eq!(format_transfer_rate(2048.0), "2.00 KB/s");
    assert_eq!(format_transfer_rate(2.5 * 1024.0 * 1024.0), "2.50 MB/s");
}

#[tokio::test]
async fn test_live_progress_bar_lifecycle_and_helpers() {
    assert!((compute_instantaneous_rps(50, 150, 0.5) - 200.0).abs() < 1e-6);
    assert_eq!(
        format_live_message(150, 200.0, 2),
        "Req: 150 | RPS: 200.0 | Err: 2"
    );

    let counters = Arc::new(LiveCounters::new());
    let cancel_token = CancellationToken::new();

    let handle = spawn_live_progress_bar(
        Duration::from_secs(5),
        Arc::clone(&counters),
        cancel_token.clone(),
    );

    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel_token.cancel();

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("live progress bar should exit promptly on cancellation")
        .expect("live progress bar task should not panic");
}
