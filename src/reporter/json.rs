//! JSON exporter writing formatted benchmark statistics and execution metadata to disk.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Deserializer, Serialize};

use crate::config::BenchmarkConfig;
use crate::metrics::stats::BenchmarkStats;

/// Complete JSON report combining execution metadata and flattened [`BenchmarkStats`].
///
/// Because `stats` is annotated with `#[serde(flatten)]`, the serialized JSON object
/// contains all top-level [`BenchmarkStats`] keys (`total_requests`, `latencies`,
/// `status_distribution`, etc.) alongside the execution metadata fields (`timestamp`,
/// `url`, `method`, `concurrency`, `configured_duration_secs`, `headers`), and can be
/// deserialized directly into either [`JsonReport`] or [`BenchmarkStats`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonReport {
    /// ISO-8601 UTC timestamp when the report was generated.
    pub timestamp: String,
    /// Target URL tested.
    pub url: String,
    /// HTTP method used.
    pub method: String,
    /// Number of concurrent virtual users.
    pub concurrency: usize,
    /// Configured target duration in seconds.
    pub configured_duration_secs: f64,
    /// Configured request headers.
    pub headers: HashMap<String, String>,
    /// Flattened benchmark summary metrics.
    #[serde(flatten)]
    pub stats: BenchmarkStats,
}

impl<'de> Deserialize<'de> for JsonReport {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct MetadataOnly {
            timestamp: String,
            url: String,
            method: String,
            concurrency: usize,
            configured_duration_secs: f64,
            #[serde(default)]
            headers: HashMap<String, String>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let meta: MetadataOnly =
            serde_json::from_value(value.clone()).map_err(serde::de::Error::custom)?;
        let stats: BenchmarkStats =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;

        Ok(JsonReport {
            timestamp: meta.timestamp,
            url: meta.url,
            method: meta.method,
            concurrency: meta.concurrency,
            configured_duration_secs: meta.configured_duration_secs,
            headers: meta.headers,
            stats,
        })
    }
}

impl JsonReport {
    /// Constructs a new [`JsonReport`] from a [`BenchmarkConfig`] and [`BenchmarkStats`].
    pub fn new(config: &BenchmarkConfig, stats: &BenchmarkStats) -> Self {
        let mut headers = HashMap::new();
        for (name, value) in &config.headers {
            if let Ok(val_str) = value.to_str() {
                headers.insert(name.as_str().to_string(), val_str.to_string());
            }
        }

        Self {
            timestamp: Utc::now().to_rfc3339(),
            url: config.url.to_string(),
            method: config.method.as_str().to_string(),
            concurrency: config.concurrency,
            configured_duration_secs: config.duration.as_secs_f64(),
            headers,
            stats: stats.clone(),
        }
    }
}

/// Serializes the benchmark report to a pretty-printed JSON string.
pub fn serialize_json_report(config: &BenchmarkConfig, stats: &BenchmarkStats) -> Result<String> {
    let report = JsonReport::new(config, stats);
    serde_json::to_string_pretty(&report).context("failed to serialize benchmark report to JSON")
}

/// Exports the benchmark report as pretty-printed JSON to `path`, creating any missing
/// parent directories automatically.
pub fn export_json_report(
    path: &Path,
    config: &BenchmarkConfig,
    stats: &BenchmarkStats,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create parent directory '{}'",
                    parent.display()
                )
            })?;
        }
    }

    let json_payload = serialize_json_report(config, stats)?;
    fs::write(path, json_payload)
        .with_context(|| format!("failed to write JSON report to '{}'", path.display()))?;

    Ok(())
}
