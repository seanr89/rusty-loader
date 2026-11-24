//! Shared format-neutral benchmark export model, redaction helpers, and disk writer.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::changelog::APP_VERSION;
use crate::config::{redact_headers, sanitize_url, BenchmarkConfig};
use crate::metrics::snapshot::TelemetrySnapshot;
use crate::metrics::stats::BenchmarkStats;
use crate::reporter::JsonReport;

/// Supported report export formats across CLI, web workbench, and macOS desktop UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Json,
    Csv,
    Pdf,
}

impl ExportFormat {
    /// Returns the lowercase file extension without a leading dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Pdf => "pdf",
        }
    }

    /// Returns the HTTP `Content-Type` header value for downloads.
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json; charset=utf-8",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Pdf => "application/pdf",
        }
    }

    /// Returns the default filename suggested by browser downloads and Save dialogs.
    pub const fn default_filename(self) -> &'static str {
        match self {
            Self::Json => "rusty-loader-report.json",
            Self::Csv => "rusty-loader-report.csv",
            Self::Pdf => "rusty-loader-report.pdf",
        }
    }

    /// Returns the static HTTP `Content-Disposition` attachment header value.
    pub const fn content_disposition(self) -> &'static str {
        match self {
            Self::Json => "attachment; filename=\"rusty-loader-report.json\"",
            Self::Csv => "attachment; filename=\"rusty-loader-report.csv\"",
            Self::Pdf => "attachment; filename=\"rusty-loader-report.pdf\"",
        }
    }

    /// Returns `(filter_label, extensions)` for native OS Save file dialogs.
    pub const fn dialog_filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Json => ("JSON Report", &["json"]),
            Self::Csv => ("CSV Report", &["csv"]),
            Self::Pdf => ("PDF Report", &["pdf"]),
        }
    }
}

/// Sanitized, format-neutral snapshot of a benchmark run ready for JSON, CSV, or PDF export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportReport {
    /// Binary semantic version when exported.
    pub app_version: String,
    /// ISO-8601 UTC timestamp when the run completed or was exported.
    pub timestamp: String,
    /// Optional preset name selected in the interactive UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_name: Option<String>,
    /// Target URL with any embedded credentials replaced by `REDACTED`.
    pub url: String,
    /// HTTP method used for requests.
    pub method: String,
    /// Target virtual user concurrency.
    pub concurrency: usize,
    /// Configured test duration in seconds.
    pub configured_duration_secs: f64,
    /// Configured ramp-up duration in seconds.
    pub ramp_up_secs: f64,
    /// Per-request timeout in milliseconds.
    pub timeout_ms: u64,
    /// Request headers with sensitive tokens/credentials replaced by `"[REDACTED]"`.
    pub headers: HashMap<String, String>,
    /// Cumulative benchmark statistics.
    pub stats: BenchmarkStats,
    /// Chronological windowed telemetry snapshots collected during execution.
    pub snapshots: Vec<TelemetrySnapshot>,
}

impl ExportReport {
    /// Builds a redacted [`ExportReport`] from validated runtime config, stats, and snapshots.
    pub fn from_config_redacted(
        config: &BenchmarkConfig,
        stats: &BenchmarkStats,
        snapshots: Vec<TelemetrySnapshot>,
        preset_name: Option<String>,
    ) -> Self {
        Self {
            app_version: APP_VERSION.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            preset_name,
            url: sanitize_url(&config.url),
            method: config.method.as_str().to_string(),
            concurrency: config.concurrency,
            configured_duration_secs: config.duration.as_secs_f64(),
            ramp_up_secs: config.ramp_up.as_secs_f64(),
            timeout_ms: config.timeout.as_millis() as u64,
            headers: redact_headers(&config.headers),
            stats: stats.clone(),
            snapshots,
        }
    }

    /// Builds a redacted [`ExportReport`] from an archived [`CompletedRunRecord`], if stats exist.
    #[cfg(feature = "gui")]
    pub fn from_completed_record(record: &crate::gui::manager::CompletedRunRecord) -> Option<Self> {
        let stats = record.stats.as_ref()?.clone();
        Some(Self {
            app_version: APP_VERSION.to_string(),
            timestamp: record.timestamp.clone(),
            preset_name: record.preset_name.clone(),
            url: record.meta.url.clone(),
            method: record.meta.method.clone(),
            concurrency: record.meta.concurrency,
            configured_duration_secs: record.meta.duration_secs,
            ramp_up_secs: record.meta.ramp_up_secs,
            timeout_ms: record.meta.timeout_ms,
            headers: record.meta.headers.clone(),
            stats,
            snapshots: record.snapshots.clone(),
        })
    }

    /// Converts this report into a redacted [`JsonReport`] preserving legacy JSON shape.
    pub fn to_json_report(&self) -> JsonReport {
        JsonReport {
            timestamp: self.timestamp.clone(),
            url: self.url.clone(),
            method: self.method.clone(),
            concurrency: self.concurrency,
            configured_duration_secs: self.configured_duration_secs,
            headers: self.headers.clone(),
            stats: self.stats.clone(),
        }
    }

    /// Serializes this report into pretty JSON bytes.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(&self.to_json_report())
            .context("failed to serialize redacted JSON benchmark report")
    }

    /// Encodes this report into bytes for the requested [`ExportFormat`].
    pub fn encode(&self, format: ExportFormat) -> Result<Vec<u8>> {
        match format {
            ExportFormat::Json => self.to_json_bytes(),
            ExportFormat::Csv => Ok(crate::reporter::csv::serialize_csv_report(self)?.into_bytes()),
            ExportFormat::Pdf => crate::reporter::pdf::generate_pdf_report(self),
        }
    }
}

/// Writes exported bytes to `path`, creating missing parent directories automatically.
pub fn write_export_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create export directory '{}'", parent.display())
            })?;
        }
    }

    fs::write(path, bytes)
        .with_context(|| format!("failed to write exported report to '{}'", path.display()))
}
