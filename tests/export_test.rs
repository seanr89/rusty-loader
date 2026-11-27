use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rusty_loader::config::BenchmarkConfig;
use rusty_loader::metrics::snapshot::{TelemetryPhase, TelemetrySnapshot, WindowLatencyMs};
use rusty_loader::metrics::stats::{BenchmarkStats, LatencyDistribution};
use rusty_loader::reporter::{
    escape_csv_formula, generate_pdf_report, pdf_sanitize_winansi, serialize_csv_report,
    ExportReport, CSV_HEADERS,
};

fn redacted_test_config(raw_url: &str) -> BenchmarkConfig {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer topsecret-jwt-value"),
    );
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_static("super-secret-api-key"),
    );
    headers.insert(
        HeaderName::from_static("cookie"),
        HeaderValue::from_static("session=secret-cookie-val"),
    );
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );

    BenchmarkConfig {
        url: Url::parse(raw_url).expect("valid test url"),
        concurrency: 64,
        duration: Duration::from_secs(10),
        method: Method::POST,
        headers,
        body: Some(r#"{"query":"ping"}"#.to_string()),
        timeout: Duration::from_secs(2),
        ramp_up: Duration::from_secs(1),
        output_json: None,
        output_csv: None,
        output_pdf: None,
        scenario: None,
    }
}

fn sample_stats(with_errors: bool) -> BenchmarkStats {
    let mut status_distribution = HashMap::new();
    status_distribution.insert(200, 950);
    status_distribution.insert(201, 25);
    status_distribution.insert(502, 15);

    let mut error_breakdown = HashMap::new();
    let transport_errors = if with_errors {
        error_breakdown.insert("RequestTimeout".to_string(), 10);
        10
    } else {
        0
    };

    BenchmarkStats {
        total_duration_secs: 2.5,
        total_requests: 1_000,
        successful_requests: 975,
        client_errors: 0,
        server_errors: 15,
        other_statuses: 0,
        transport_errors,
        requests_per_sec: 400.0,
        total_bytes: 256_000,
        bytes_per_sec: 102_400.0,
        latencies: LatencyDistribution {
            min_ms: 0.8,
            mean_ms: 14.2,
            p50_ms: 11.4,
            p90_ms: 25.0,
            p95_ms: 33.6,
            p99_ms: 48.9,
            p99_9_ms: 71.2,
            max_ms: 95.4,
            std_dev_ms: 8.3,
        },
        status_distribution,
        error_breakdown,
        scenario_stats: None,
    }
}

fn sample_snapshots(count: usize) -> Vec<TelemetrySnapshot> {
    (0..count)
        .map(|idx| {
            let elapsed = (idx as f64 + 1.0) * 0.25;
            let total = ((idx as u64) + 1) * 100;
            TelemetrySnapshot {
                timestamp_ms: 1_795_555_000_000 + (idx as u64 * 250),
                elapsed_secs: elapsed,
                duration_secs: 2.5,
                phase: if idx == 0 {
                    TelemetryPhase::RampingUp
                } else {
                    TelemetryPhase::Running
                },
                active_workers: (idx + 1).min(64) * 4,
                total_requests: total,
                window_rps: 380.0 + (idx as f64 * 12.5),
                avg_rps: total as f64 / elapsed,
                bytes_per_sec: 98_000.0,
                total_bytes: total * 256,
                successful_requests: total.saturating_sub(idx as u64 * 2),
                client_errors: idx as u64,
                server_errors: if idx % 2 == 0 { 1 } else { 0 },
                other_statuses: 0,
                transport_errors: if idx > 1 { 1 } else { 0 },
                window_latency_ms: WindowLatencyMs {
                    min: 0.9,
                    mean: 12.0 + idx as f64,
                    p50: 10.0 + idx as f64,
                    p90: 21.0 + idx as f64,
                    p95: 29.0 + idx as f64,
                    p99: 44.0 + idx as f64,
                    max: 62.0 + idx as f64 * 2.0,
                },
                status_distribution: HashMap::new(),
                error_breakdown: HashMap::new(),
                step_snapshots: None,
                journey_snapshot: None,
            }
        })
        .collect()
}

#[test]
fn test_csv_export_rectangular_schema_redaction_and_ticks() {
    let config = redacted_test_config("https://svc_user:cred_secret@api.example.com/v1/query");
    let stats = sample_stats(true);
    let snapshots = sample_snapshots(3);
    let report =
        ExportReport::from_config_redacted(&config, &stats, snapshots, Some("Smoke".to_string()));

    let csv_text = serialize_csv_report(&report).expect("CSV serialization succeeds");
    assert!(!csv_text.contains("cred_secret"));
    assert!(!csv_text.contains("topsecret-jwt-value"));
    assert!(!csv_text.contains("super-secret-api-key"));
    assert!(!csv_text.contains("secret-cookie-val"));

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_text.as_bytes());
    let rows: Vec<Vec<String>> = reader
        .records()
        .map(|res| {
            res.expect("valid CSV record")
                .iter()
                .map(str::to_string)
                .collect()
        })
        .collect();

    assert_eq!(rows.len(), 5, "expected 1 header + 3 ticks + 1 summary row");
    assert_eq!(
        rows[0].iter().map(String::as_str).collect::<Vec<_>>(),
        CSV_HEADERS.to_vec()
    );

    for row in &rows {
        assert_eq!(row.len(), 26, "every CSV row must be 26 columns wide");
    }

    // Tick rows have window_rps populated and p99_9_ms left blank
    assert_eq!(rows[1][0], "tick");
    assert_eq!(rows[1][3], "ramping_up");
    assert!(rows[1][4].contains("REDACTED:REDACTED@api.example.com"));
    assert!(!rows[1][9].is_empty());
    assert_eq!(rows[1][24], "");

    // Final summary row has cumulative metrics & active_workers == 0
    let summary = &rows[4];
    assert_eq!(summary[0], "summary");
    assert_eq!(summary[3], "completed");
    assert_eq!(summary[7], "0");
    assert_eq!(summary[8], "1000");
    assert_eq!(summary[9], "");
    assert_eq!(summary[10], "400.000");
    assert_eq!(summary[24], "71.200");
}

#[test]
fn test_csv_export_zero_snapshots_and_formula_injection_defense() {
    let config = redacted_test_config("https://api.example.com/v1/ping");
    let stats = sample_stats(false);
    let mut report = ExportReport::from_config_redacted(&config, &stats, Vec::new(), None);
    report.url = "=CMD|'/C calc'!A0".to_string();
    report.method = "@SUM(1,2)".to_string();

    assert_eq!(escape_csv_formula("=1+1").as_ref(), "'=1+1");
    assert_eq!(escape_csv_formula("+cmd").as_ref(), "'+cmd");
    assert_eq!(escape_csv_formula("-cmd").as_ref(), "'-cmd");
    assert_eq!(escape_csv_formula("@SUM(A1:A2)").as_ref(), "'@SUM(A1:A2)");
    assert_eq!(escape_csv_formula("\ttab").as_ref(), "'\ttab");
    assert_eq!(escape_csv_formula("\rcr").as_ref(), "'\rcr");
    assert_eq!(
        escape_csv_formula("https://safe.example.com").as_ref(),
        "https://safe.example.com"
    );

    let csv_text = serialize_csv_report(&report).expect("CSV serialization succeeds");
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_text.as_bytes());
    let rows: Vec<Vec<String>> = reader
        .records()
        .map(|r| r.unwrap().iter().map(str::to_string).collect())
        .collect();

    assert_eq!(rows.len(), 2, "expected header + summary row for zero ticks");
    assert_eq!(rows[1][0], "summary");
    assert_eq!(rows[1][4], "'=CMD|'/C calc'!A0");
    assert_eq!(rows[1][5], "'@SUM(1,2)");
    // Numeric cell must remain unprefixed
    assert_eq!(rows[1][10], "400.000");
}

#[test]
fn test_pdf_export_structure_page_count_redaction_and_winansi() {
    let mut config = redacted_test_config("https://svc_user:cred_secret@api.example.com/v1/query");
    // Exercise header overflow clamping (>6 headers) and status code overflow clamping (>10 statuses)
    for i in 0..10 {
        config.headers.insert(
            HeaderName::from_bytes(format!("x-extra-{i}").as_bytes()).unwrap(),
            HeaderValue::from_str(&format!("val-{i}")).unwrap(),
        );
    }

    let mut stats = sample_stats(true);
    for code in 400u16..415u16 {
        stats.status_distribution.insert(code, u64::from(code - 399));
    }

    // 1. Single-tick / empty-snapshot PDF emits exactly 1 page (/Count 1)
    let single_page_report = ExportReport::from_config_redacted(
        &config,
        &stats,
        sample_snapshots(1),
        Some("Canary — 12µs 🚀".to_string()),
    );
    let pdf1 = generate_pdf_report(&single_page_report).expect("single-page PDF generates");
    assert!(pdf1.starts_with(b"%PDF-1.7"));
    let pdf1_text = String::from_utf8_lossy(&pdf1);
    assert!(pdf1_text.trim_end().ends_with("%%EOF"));
    assert!(pdf1_text.contains("/Count 1"));
    assert!(pdf1_text.contains("Page 1 of 1"));
    assert!(pdf1_text.contains("[REDACTED]"));
    assert!(pdf1_text.contains("(+"));
    assert!(!pdf1_text.contains("cred_secret"));
    assert!(!pdf1_text.contains("topsecret-jwt-value"));
    assert!(!pdf1_text.contains("super-secret-api-key"));
    assert!(!pdf1_text.contains("secret-cookie-val"));

    // 2. Multi-tick PDF (>=2 snapshots) emits 2 pages (/Count 2) with vector telemetry charts
    let two_page_report =
        ExportReport::from_config_redacted(&config, &stats, sample_snapshots(5), None);
    let pdf2 = generate_pdf_report(&two_page_report).expect("two-page PDF generates");
    let pdf2_text = String::from_utf8_lossy(&pdf2);
    assert!(pdf2_text.contains("/Count 2"));
    assert!(pdf2_text.contains("Page 1 of 2"));
    assert!(pdf2_text.contains("Page 2 of 2"));
    assert!(pdf2_text.contains("Throughput & Active Virtual Users"));
    assert!(pdf2_text.contains("Windowed HDR Latency Percentiles"));
    assert!(pdf2_text.contains("Per-Tick Responses & Errors by Class"));

    // 3. WinAnsi string normalizer preserves ASCII and replaces non-WinAnsi safely
    let sanitized = pdf_sanitize_winansi("Latency — 12µs ‘ok’ 🚀");
    assert_eq!(String::from_utf8_lossy(&sanitized), "Latency -- 12us 'ok' ?");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cli_end_to_end_exports_json_csv_and_pdf() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bench"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&mock_server)
        .await;

    let tmp_dir = TempDir::new().expect("temp dir");
    let json_path = tmp_dir.path().join("out/report.json");
    let csv_path = tmp_dir.path().join("out/report.csv");
    let pdf_path = tmp_dir.path().join("out/report.pdf");
    let target_url = format!("{}/bench", mock_server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_rusty-loader"))
        .args([
            "--url",
            &target_url,
            "--concurrency",
            "4",
            "--duration",
            "650ms",
            "--header",
            "Authorization: Bearer cli-secret-token",
            "--output-json",
            json_path.to_str().unwrap(),
            "--output-csv",
            csv_path.to_str().unwrap(),
            "--output-pdf",
            pdf_path.to_str().unwrap(),
        ])
        .output()
        .expect("execute rusty-loader binary");

    assert!(
        output.status.success(),
        "rusty-loader exited with {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // 1. JSON file exists & parses cleanly
    let json_raw = std::fs::read_to_string(&json_path).expect("read exported json");
    let json_val: serde_json::Value = serde_json::from_str(&json_raw).expect("valid json");
    assert!(json_val["total_requests"].as_u64().unwrap_or(0) > 0);

    // 2. CSV file exists, parses 26 columns, includes >=1 tick from TelemetryHub + summary, and redacts header
    let csv_raw = std::fs::read_to_string(&csv_path).expect("read exported csv");
    assert!(!csv_raw.contains("cli-secret-token"));
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_raw.as_bytes());
    let rows: Vec<Vec<String>> = reader
        .records()
        .map(|r| r.unwrap().iter().map(str::to_string).collect())
        .collect();
    assert!(
        rows.len() >= 3,
        "expected header + >=1 telemetry tick + summary row, got {} rows",
        rows.len()
    );
    assert_eq!(rows.last().unwrap()[0], "summary");
    for row in &rows {
        assert_eq!(row.len(), 26);
    }

    // 3. PDF file exists, starts with %PDF-1.7, and redacts secret
    let pdf_raw = std::fs::read(&pdf_path).expect("read exported pdf");
    assert!(pdf_raw.starts_with(b"%PDF-1.7"));
    let pdf_text = String::from_utf8_lossy(&pdf_raw);
    assert!(pdf_text.trim_end().ends_with("%%EOF"));
    assert!(!pdf_text.contains("cli-secret-token"));
}
