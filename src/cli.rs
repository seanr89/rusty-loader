use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

/// High-performance, concurrent API load testing CLI written in Rust.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "rusty-loader",
    version,
    about = "High-performance, concurrent API load testing CLI written in Rust",
    long_about = None
)]
pub struct Cli {
    /// Target endpoint URL (e.g. https://api.example.com/v1/ping)
    #[arg(short = 'u', long = "url", required = true)]
    pub url: String,

    /// Number of concurrent virtual users / workers
    #[arg(short = 'c', long = "concurrency", default_value_t = 700)]
    pub concurrency: usize,

    /// Total benchmark duration (e.g. 30s, 2m, 1000ms, or raw seconds like 60)
    #[arg(
        short = 'd',
        long = "duration",
        default_value = "30s",
        value_parser = parse_duration
    )]
    pub duration: Duration,

    /// HTTP request method
    #[arg(
        short = 'm',
        long = "method",
        default_value = "GET",
        ignore_case = true
    )]
    pub method: HttpMethod,

    /// Key-value request headers, repeatable (e.g. -H "Authorization: Bearer xyz")
    #[arg(short = 'H', long = "header")]
    pub headers: Vec<String>,

    /// Optional request body as raw string or file path prefixed with '@' (e.g. @payload.json)
    #[arg(short = 'b', long = "body")]
    pub body: Option<String>,

    /// Request timeout (e.g. 5000ms, 5s, or raw milliseconds like 2000)
    #[arg(
        short = 't',
        long = "timeout",
        default_value = "5000ms",
        value_parser = parse_timeout
    )]
    pub timeout: Duration,

    /// Ramp-up duration to incrementally reach target concurrency (e.g. 10s, 1m, or raw seconds)
    #[arg(
        long = "ramp-up",
        default_value = "0s",
        value_parser = parse_duration
    )]
    pub ramp_up: Duration,

    /// Optional file path to write the final JSON benchmark report
    #[arg(long = "output-json")]
    pub output_json: Option<PathBuf>,
}

/// Supported HTTP methods for load testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[clap(rename_all = "UPPER")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
}

impl From<HttpMethod> for reqwest::Method {
    fn from(method: HttpMethod) -> Self {
        match method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Head => reqwest::Method::HEAD,
        }
    }
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
        };
        write!(f, "{s}")
    }
}

impl FromStr for HttpMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "GET" => Ok(HttpMethod::Get),
            "POST" => Ok(HttpMethod::Post),
            "PUT" => Ok(HttpMethod::Put),
            "DELETE" => Ok(HttpMethod::Delete),
            "PATCH" => Ok(HttpMethod::Patch),
            "HEAD" => Ok(HttpMethod::Head),
            other => Err(format!(
                "unsupported HTTP method '{other}': expected GET, POST, PUT, DELETE, PATCH, or HEAD"
            )),
        }
    }
}

/// Parses a duration string (`"30s"`, `"2m"`, `"500ms"`, `"1h"`) or raw integer seconds (`"60"`).
pub fn parse_duration(input: &str) -> Result<Duration, String> {
    parse_duration_with_default_unit(input, DefaultUnit::Seconds)
}

/// Parses a timeout duration string (`"5000ms"`, `"5s"`, `"1m"`) or raw integer milliseconds (`"2000"`).
pub fn parse_timeout(input: &str) -> Result<Duration, String> {
    parse_duration_with_default_unit(input, DefaultUnit::Milliseconds)
}

#[derive(Debug, Clone, Copy)]
enum DefaultUnit {
    Seconds,
    Milliseconds,
}

fn parse_duration_with_default_unit(
    input: &str,
    default_unit: DefaultUnit,
) -> Result<Duration, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("duration cannot be empty".to_string());
    }

    if let Some(num_str) = trimmed.strip_suffix("ms") {
        let val = parse_u64(num_str, trimmed)?;
        Ok(Duration::from_millis(val))
    } else if let Some(num_str) = trimmed.strip_suffix('s') {
        let val = parse_u64(num_str, trimmed)?;
        Ok(Duration::from_secs(val))
    } else if let Some(num_str) = trimmed.strip_suffix('m') {
        let val = parse_u64(num_str, trimmed)?;
        val.checked_mul(60)
            .map(Duration::from_secs)
            .ok_or_else(|| format!("duration overflow in '{trimmed}'"))
    } else if let Some(num_str) = trimmed.strip_suffix('h') {
        let val = parse_u64(num_str, trimmed)?;
        val.checked_mul(3600)
            .map(Duration::from_secs)
            .ok_or_else(|| format!("duration overflow in '{trimmed}'"))
    } else {
        let val = parse_u64(trimmed, trimmed)?;
        match default_unit {
            DefaultUnit::Seconds => Ok(Duration::from_secs(val)),
            DefaultUnit::Milliseconds => Ok(Duration::from_millis(val)),
        }
    }
}

fn parse_u64(num_str: &str, original: &str) -> Result<u64, String> {
    let num_trimmed = num_str.trim();
    if num_trimmed.is_empty() {
        return Err(format!("missing numeric value in duration '{original}'"));
    }
    num_trimmed.parse::<u64>().map_err(|_| {
        format!(
            "invalid duration '{original}': expected a non-negative integer followed by optional unit (ms, s, m, h)"
        )
    })
}

/// Parses a single `"Key: Value"` header string into `(HeaderName, HeaderValue)`.
pub fn parse_header(raw: &str) -> Result<(HeaderName, HeaderValue), String> {
    let (key, value) = raw.split_once(':').ok_or_else(|| {
        format!("malformed header '{raw}': expected 'Key: Value' format with a ':' separator")
    })?;

    let key_trimmed = key.trim();
    if key_trimmed.is_empty() {
        return Err(format!("malformed header '{raw}': header name cannot be empty"));
    }

    let header_name = HeaderName::from_str(key_trimmed)
        .map_err(|e| format!("invalid header name '{key_trimmed}': {e}"))?;

    let value_trimmed = value.trim();
    let header_value = HeaderValue::from_str(value_trimmed)
        .map_err(|e| format!("invalid header value for '{key_trimmed}': {e}"))?;

    Ok((header_name, header_value))
}
