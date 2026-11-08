//! Formatted multi-table terminal summary using `comfy-table` and `colored`.

use colored::{ColoredString, Colorize};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Table};
use reqwest::StatusCode;

use crate::config::BenchmarkConfig;
use crate::metrics::stats::BenchmarkStats;

/// Formats a latency value in milliseconds with 2 decimal places (e.g., `"1.24 ms"`).
pub fn format_latency(ms: f64) -> String {
    format!("{ms:.2} ms")
}

/// Colorizes a latency value according to thresholds:
/// - `< 50.0 ms`: Green
/// - `50.0..=200.0 ms`: Yellow
/// - `> 200.0 ms`: Red
pub fn colorize_latency(ms: f64) -> ColoredString {
    let formatted = format_latency(ms);
    if ms < 50.0 {
        formatted.green()
    } else if ms <= 200.0 {
        formatted.yellow()
    } else {
        formatted.red()
    }
}

/// Formats a latency value in milliseconds as an ANSI-colored string.
pub fn format_latency_colored(ms: f64) -> String {
    colorize_latency(ms).to_string()
}

/// Formats a byte count into human-readable units (`B`, `KB`, `MB`, `GB`).
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.2} KB", b / KB)
    } else if b < GB {
        format!("{:.2} MB", b / MB)
    } else {
        format!("{:.2} GB", b / GB)
    }
}

/// Formats a byte transfer rate in bytes/sec into human-readable units (`B/s`, `KB/s`, `MB/s`, `GB/s`).
pub fn format_transfer_rate(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    if bytes_per_sec < KB {
        format!("{bytes_per_sec:.2} B/s")
    } else if bytes_per_sec < MB {
        format!("{:.2} KB/s", bytes_per_sec / KB)
    } else if bytes_per_sec < GB {
        format!("{:.2} MB/s", bytes_per_sec / MB)
    } else {
        format!("{:.2} GB/s", bytes_per_sec / GB)
    }
}

/// Builds Table 1: Benchmark Overview & Throughput.
pub fn build_overview_table(config: &BenchmarkConfig, stats: &BenchmarkStats) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![Cell::new("Metric"), Cell::new("Value")]);

    let total_errors = stats.client_errors + stats.server_errors + stats.transport_errors;
    let (success_pct, error_pct) = if stats.total_requests > 0 {
        (
            (stats.successful_requests as f64 / stats.total_requests as f64) * 100.0,
            (total_errors as f64 / stats.total_requests as f64) * 100.0,
        )
    } else {
        (0.0, 0.0)
    };

    table.add_row(vec![
        Cell::new("Target URL"),
        Cell::new(config.url.as_str()),
    ]);
    table.add_row(vec![
        Cell::new("Method"),
        Cell::new(config.method.as_str()),
    ]);
    table.add_row(vec![
        Cell::new("Concurrency (VUs)"),
        Cell::new(config.concurrency.to_string()),
    ]);
    table.add_row(vec![
        Cell::new("Total Duration"),
        Cell::new(format!("{:.2} s", stats.total_duration_secs)),
    ]);
    table.add_row(vec![
        Cell::new("Total Requests"),
        Cell::new(stats.total_requests.to_string()),
    ]);
    table.add_row(vec![
        Cell::new("Successful (2xx)"),
        Cell::new(format!(
            "{} ({success_pct:.2}%)",
            stats.successful_requests
        )),
    ]);
    table.add_row(vec![
        Cell::new("Errors (4xx/5xx/Net)"),
        Cell::new(format!("{total_errors} ({error_pct:.2}%)")),
    ]);
    table.add_row(vec![
        Cell::new("Throughput (Avg RPS)"),
        Cell::new(format!("{:.1} req/s", stats.requests_per_sec)),
    ]);
    table.add_row(vec![
        Cell::new("Total Data Transferred"),
        Cell::new(format_bytes(stats.total_bytes)),
    ]);
    table.add_row(vec![
        Cell::new("Data Transfer Rate"),
        Cell::new(format_transfer_rate(stats.bytes_per_sec)),
    ]);

    table
}

/// Builds Table 2: Latency Percentiles (HDR Histogram).
pub fn build_latency_table(stats: &BenchmarkStats) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![Cell::new("Percentile"), Cell::new("Latency")]);

    let lat = &stats.latencies;
    let rows = [
        ("Min", format_latency_colored(lat.min_ms)),
        ("Mean", format_latency_colored(lat.mean_ms)),
        ("Median (p50)", format_latency_colored(lat.p50_ms)),
        ("p90", format_latency_colored(lat.p90_ms)),
        ("p95", format_latency_colored(lat.p95_ms)),
        ("p99", format_latency_colored(lat.p99_ms)),
        ("p99.9", format_latency_colored(lat.p99_9_ms)),
        ("Max", format_latency_colored(lat.max_ms)),
        ("Std Dev", format_latency_colored(lat.std_dev_ms)),
    ];

    for (label, value) in rows {
        table.add_row(vec![Cell::new(label), Cell::new(value)]);
    }

    table
}

/// Builds Table 3: HTTP Status Code Breakdown.
pub fn build_status_table(stats: &BenchmarkStats) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Status Code"),
            Cell::new("Description"),
            Cell::new("Count"),
            Cell::new("Percentage"),
        ]);

    let mut codes: Vec<(&u16, &u64)> = stats.status_distribution.iter().collect();
    codes.sort_by_key(|(code, _)| **code);

    for (code, count) in codes {
        let description = StatusCode::from_u16(*code)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or("Unknown");
        let pct = if stats.total_requests > 0 {
            (*count as f64 / stats.total_requests as f64) * 100.0
        } else {
            0.0
        };
        table.add_row(vec![
            Cell::new(code.to_string()),
            Cell::new(description),
            Cell::new(count.to_string()),
            Cell::new(format!("{pct:.2}%")),
        ]);
    }

    table
}

/// Builds Table 4 (Conditional): Error Breakdown when transport/connection errors occurred.
pub fn build_error_table(stats: &BenchmarkStats) -> Option<Table> {
    if stats.error_breakdown.is_empty() && stats.transport_errors == 0 {
        return None;
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Error Category"),
            Cell::new("Count"),
            Cell::new("Percentage"),
        ]);

    if stats.error_breakdown.is_empty() {
        let pct = if stats.total_requests > 0 {
            (stats.transport_errors as f64 / stats.total_requests as f64) * 100.0
        } else {
            0.0
        };
        table.add_row(vec![
            Cell::new("TransportError"),
            Cell::new(stats.transport_errors.to_string()),
            Cell::new(format!("{pct:.2}%")),
        ]);
    } else {
        let mut errors: Vec<(&String, &u64)> = stats.error_breakdown.iter().collect();
        errors.sort_by(|(a_cat, a_cnt), (b_cat, b_cnt)| {
            b_cnt.cmp(a_cnt).then_with(|| a_cat.cmp(b_cat))
        });

        for (category, count) in errors {
            let pct = if stats.total_requests > 0 {
                (*count as f64 / stats.total_requests as f64) * 100.0
            } else {
                0.0
            };
            table.add_row(vec![
                Cell::new(category),
                Cell::new(count.to_string()),
                Cell::new(format!("{pct:.2}%")),
            ]);
        }
    }

    Some(table)
}

/// Renders all benchmark summary tables into a single formatted string.
pub fn render_summary_tables(config: &BenchmarkConfig, stats: &BenchmarkStats) -> String {
    let mut out = String::new();

    out.push_str("=== Benchmark Overview & Throughput ===\n");
    out.push_str(&build_overview_table(config, stats).to_string());
    out.push_str("\n\n");

    out.push_str("=== Latency Percentiles (HDR Histogram) ===\n");
    out.push_str(&build_latency_table(stats).to_string());
    out.push_str("\n\n");

    out.push_str("=== HTTP Status Code Breakdown ===\n");
    out.push_str(&build_status_table(stats).to_string());

    if let Some(error_table) = build_error_table(stats) {
        out.push_str("\n\n=== Error Breakdown ===\n");
        out.push_str(&error_table.to_string());
    }

    out.push('\n');
    out
}

/// Prints the rendered benchmark summary tables to stdout.
pub fn print_summary_tables(config: &BenchmarkConfig, stats: &BenchmarkStats) {
    print!("{}", render_summary_tables(config, stats));
}
