//! Terminal live telemetry, summary tables, and JSON report exporter.

pub mod json;
pub mod live;
pub mod table;

use anyhow::Result;

use crate::config::BenchmarkConfig;
use crate::metrics::stats::BenchmarkStats;

pub use json::{export_json_report, serialize_json_report, JsonReport};
pub use live::{
    compute_instantaneous_rps, create_progress_bar, format_live_message, spawn_live_progress_bar,
};
pub use table::{
    build_error_table, build_latency_table, build_overview_table, build_status_table,
    colorize_latency, format_bytes, format_latency, format_latency_colored, format_transfer_rate,
    print_summary_tables, render_summary_tables,
};

/// Prints the terminal summary tables and writes the optional JSON report if configured.
pub fn report_results(config: &BenchmarkConfig, stats: &BenchmarkStats) -> Result<()> {
    print_summary_tables(config, stats);

    if let Some(ref output_path) = config.output_json {
        export_json_report(output_path, config, stats)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::stats::LatencyDistribution;
    use reqwest::header::HeaderMap;
    use reqwest::{Method, Url};
    use std::collections::HashMap;
    use std::time::Duration;

    fn sample_config() -> BenchmarkConfig {
        BenchmarkConfig {
            url: Url::parse("https://api.example.com/v1/ping").expect("valid url"),
            concurrency: 700,
            duration: Duration::from_secs(60),
            method: Method::GET,
            headers: HeaderMap::new(),
            body: None,
            timeout: Duration::from_secs(5),
            ramp_up: Duration::ZERO,
            output_json: None,
        }
    }

    fn sample_stats() -> BenchmarkStats {
        let mut status_distribution = HashMap::new();
        status_distribution.insert(200, 124_480);
        status_distribution.insert(502, 20);

        BenchmarkStats {
            total_duration_secs: 60.0,
            total_requests: 124_500,
            successful_requests: 124_480,
            client_errors: 0,
            server_errors: 20,
            other_statuses: 0,
            transport_errors: 0,
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
                max_ms: 120.50,
                std_dev_ms: 3.15,
            },
            status_distribution,
            error_breakdown: HashMap::new(),
        }
    }

    #[test]
    fn test_live_rps_and_message_formatting() {
        let rps = compute_instantaneous_rps(100, 350, 0.1);
        assert!((rps - 2500.0).abs() < 1e-6);
        assert_eq!(compute_instantaneous_rps(100, 100, 0.0), 0.0);

        let msg = format_live_message(1250, 2500.0, 3);
        assert_eq!(msg, "Req: 1250 | RPS: 2500.0 | Err: 3");
    }

    #[test]
    fn test_render_summary_tables_contains_key_metrics() {
        let config = sample_config();
        let stats = sample_stats();
        let rendered = render_summary_tables(&config, &stats);

        assert!(rendered.contains("https://api.example.com/v1/ping"));
        assert!(rendered.contains("2075.0 req/s"));
        assert!(rendered.contains("Median (p50)"));
        assert!(rendered.contains("p99.9"));
        assert!(rendered.contains("200"));
        assert!(rendered.contains("OK"));
        assert!(rendered.contains("502"));
        assert!(rendered.contains("Bad Gateway"));
        assert!(!rendered.contains("=== Error Breakdown ==="));
    }

    #[test]
    fn test_json_serialization_roundtrip() {
        let config = sample_config();
        let stats = sample_stats();
        let json_str = serialize_json_report(&config, &stats).expect("serialize json");

        let parsed_stats: BenchmarkStats =
            serde_json::from_str(&json_str).expect("deserialize into BenchmarkStats");
        assert_eq!(parsed_stats, stats);

        let parsed_report: JsonReport =
            serde_json::from_str(&json_str).expect("deserialize into JsonReport");
        assert_eq!(parsed_report.url, "https://api.example.com/v1/ping");
        assert_eq!(parsed_report.concurrency, 700);
        assert_eq!(parsed_report.stats, stats);
    }
}
