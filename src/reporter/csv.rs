//! RFC 4180 CSV exporter emitting chronological windowed ticks plus a final summary row.

use std::borrow::Cow;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::DateTime;

use crate::metrics::snapshot::{TelemetryPhase, TelemetrySnapshot};
use crate::reporter::export::{write_export_bytes, ExportReport};

/// Fixed 26-column rectangular CSV header schema.
pub const CSV_HEADERS: [&str; 26] = [
    "record_type",
    "timestamp_ms",
    "elapsed_secs",
    "phase",
    "url",
    "method",
    "concurrency",
    "active_workers",
    "total_requests",
    "window_rps",
    "avg_rps",
    "bytes_per_sec",
    "total_bytes",
    "successful_requests",
    "client_errors",
    "server_errors",
    "other_statuses",
    "transport_errors",
    "min_ms",
    "mean_ms",
    "p50_ms",
    "p90_ms",
    "p95_ms",
    "p99_ms",
    "p99_9_ms",
    "max_ms",
];

/// Neutralizes spreadsheet formula injection (CWE-1236) on untrusted string cells.
///
/// Any non-empty cell beginning with `=`, `+`, `-`, `@`, `\t`, or `\r` is prefixed
/// with a single quote (`'`). Numeric columns should be formatted directly without this helper.
pub fn escape_csv_formula(value: &str) -> Cow<'_, str> {
    if matches!(
        value.as_bytes().first(),
        Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r')
    ) {
        Cow::Owned(format!("'{value}"))
    } else {
        Cow::Borrowed(value)
    }
}

fn phase_label(phase: TelemetryPhase) -> &'static str {
    match phase {
        TelemetryPhase::Idle => "idle",
        TelemetryPhase::RampingUp => "ramping_up",
        TelemetryPhase::Running => "running",
        TelemetryPhase::Draining => "draining",
        TelemetryPhase::Completed => "completed",
        TelemetryPhase::Failed => "failed",
    }
}

fn parse_summary_timestamp_ms(timestamp: &str) -> u64 {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .and_then(|dt| u64::try_from(dt.timestamp_millis()).ok())
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
fn write_csv_row(
    writer: &mut csv::Writer<Vec<u8>>,
    record_type: &str,
    timestamp_ms: u64,
    elapsed_secs: f64,
    phase: &str,
    url_cell: &str,
    method_cell: &str,
    concurrency_cell: &str,
    active_workers: usize,
    total_requests: u64,
    window_rps: Option<f64>,
    avg_rps: f64,
    bytes_per_sec: f64,
    total_bytes: u64,
    successful_requests: u64,
    client_errors: u64,
    server_errors: u64,
    other_statuses: u64,
    transport_errors: u64,
    min_ms: f64,
    mean_ms: f64,
    p50_ms: f64,
    p90_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    p99_9_ms: Option<f64>,
    max_ms: f64,
) -> Result<()> {
    let window_rps_cell = window_rps.map(|v| format!("{v:.3}")).unwrap_or_default();
    let p99_9_cell = p99_9_ms.map(|v| format!("{v:.3}")).unwrap_or_default();
    let escaped_phase = escape_csv_formula(phase);

    writer
        .write_record([
            record_type,
            &timestamp_ms.to_string(),
            &format!("{elapsed_secs:.6}"),
            escaped_phase.as_ref(),
            url_cell,
            method_cell,
            concurrency_cell,
            &active_workers.to_string(),
            &total_requests.to_string(),
            &window_rps_cell,
            &format!("{avg_rps:.3}"),
            &format!("{bytes_per_sec:.3}"),
            &total_bytes.to_string(),
            &successful_requests.to_string(),
            &client_errors.to_string(),
            &server_errors.to_string(),
            &other_statuses.to_string(),
            &transport_errors.to_string(),
            &format!("{min_ms:.3}"),
            &format!("{mean_ms:.3}"),
            &format!("{p50_ms:.3}"),
            &format!("{p90_ms:.3}"),
            &format!("{p95_ms:.3}"),
            &format!("{p99_ms:.3}"),
            &p99_9_cell,
            &format!("{max_ms:.3}"),
        ])
        .context("failed to write CSV benchmark row")
}

/// Serializes an [`ExportReport`] into a strictly 26-column RFC 4180 CSV string.
pub fn serialize_csv_report(report: &ExportReport) -> Result<String> {
    let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
    writer
        .write_record(CSV_HEADERS)
        .context("failed to write CSV header row")?;

    let url_cell = escape_csv_formula(&report.url);
    let method_cell = escape_csv_formula(&report.method);
    let concurrency_cell = report.concurrency.to_string();

    for snap in &report.snapshots {
        write_tick_row(&mut writer, snap, &url_cell, &method_cell, &concurrency_cell)?;
    }

    let summary_ts = parse_summary_timestamp_ms(&report.timestamp);
    let stats = &report.stats;
    write_csv_row(
        &mut writer,
        "summary",
        summary_ts,
        stats.total_duration_secs,
        "completed",
        url_cell.as_ref(),
        method_cell.as_ref(),
        &concurrency_cell,
        0,
        stats.total_requests,
        None,
        stats.requests_per_sec,
        stats.bytes_per_sec,
        stats.total_bytes,
        stats.successful_requests,
        stats.client_errors,
        stats.server_errors,
        stats.other_statuses,
        stats.transport_errors,
        stats.latencies.min_ms,
        stats.latencies.mean_ms,
        stats.latencies.p50_ms,
        stats.latencies.p90_ms,
        stats.latencies.p95_ms,
        stats.latencies.p99_ms,
        Some(stats.latencies.p99_9_ms),
        stats.latencies.max_ms,
    )?;

    let bytes = writer
        .into_inner()
        .context("failed to flush CSV writer buffer")?;
    String::from_utf8(bytes).context("generated CSV output contained invalid UTF-8")
}

fn write_tick_row(
    writer: &mut csv::Writer<Vec<u8>>,
    snap: &TelemetrySnapshot,
    url_cell: &str,
    method_cell: &str,
    concurrency_cell: &str,
) -> Result<()> {
    let lat = &snap.window_latency_ms;
    write_csv_row(
        writer,
        "tick",
        snap.timestamp_ms,
        snap.elapsed_secs,
        phase_label(snap.phase),
        url_cell,
        method_cell,
        concurrency_cell,
        snap.active_workers,
        snap.total_requests,
        Some(snap.window_rps),
        snap.avg_rps,
        snap.bytes_per_sec,
        snap.total_bytes,
        snap.successful_requests,
        snap.client_errors,
        snap.server_errors,
        snap.other_statuses,
        snap.transport_errors,
        lat.min,
        lat.mean,
        lat.p50,
        lat.p90,
        lat.p95,
        lat.p99,
        None,
        lat.max,
    )
}

/// Serializes and writes an [`ExportReport`] as CSV to `path`, creating parent directories.
pub fn export_csv_report(path: &Path, report: &ExportReport) -> Result<()> {
    let csv = serialize_csv_report(report)?;
    write_export_bytes(path, csv.as_bytes())
}
