use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cli::{parse_duration, parse_header, parse_timeout, Cli, HttpMethod};
use crate::os::OsLimits;

/// Errors that can occur when validating and constructing [`BenchmarkConfig`].
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid target URL '{url}': {reason}")]
    InvalidUrl { url: String, reason: String },

    #[error("unsupported URL scheme '{scheme}': only 'http' and 'https' are supported")]
    UnsupportedScheme { scheme: String },

    #[error("concurrency must be greater than 0 (got {0})")]
    InvalidConcurrency(usize),

    #[error("benchmark duration must be greater than 0ms")]
    InvalidDuration,

    #[error("invalid duration: {0}")]
    InvalidDurationFormat(String),

    #[error("request timeout must be greater than 0ms")]
    InvalidTimeout,

    #[error("invalid timeout: {0}")]
    InvalidTimeoutFormat(String),

    #[error("ramp-up duration ({ramp_up:?}) cannot exceed total benchmark duration ({duration:?})")]
    RampUpExceedsDuration {
        ramp_up: Duration,
        duration: Duration,
    },

    #[error("invalid ramp-up duration: {0}")]
    InvalidRampUpFormat(String),

    #[error("{0}")]
    InvalidHeader(String),

    #[error("failed to read request body file '{path}': {source}")]
    BodyFileReadError {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("body file path after '@' cannot be empty")]
    EmptyBodyFilePath,

    #[error("invalid GUI bind IP address '{addr}': {reason}")]
    InvalidGuiBindAddress { addr: String, reason: String },
}

fn default_true() -> bool {
    true
}

fn default_method() -> HttpMethod {
    HttpMethod::Get
}

fn default_concurrency() -> usize {
    700
}

fn default_duration_str() -> String {
    "30s".to_string()
}

fn default_ramp_up_str() -> String {
    "0s".to_string()
}

fn default_timeout_str() -> String {
    "5000ms".to_string()
}

/// Serializable HTTP header entry used by interactive UI configs and saved presets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub name: String,
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// How the request payload body is specified in [`BenchmarkConfigInput`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyMode {
    #[default]
    None,
    Raw,
    File,
}

/// Serializable, pre-validation benchmark configuration shared by UI forms, presets, and CLI adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkConfigInput {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: HttpMethod,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_duration_str")]
    pub duration: String,
    #[serde(default = "default_ramp_up_str")]
    pub ramp_up: String,
    #[serde(default = "default_timeout_str")]
    pub timeout: String,
    #[serde(default)]
    pub headers: Vec<HeaderEntry>,
    #[serde(default)]
    pub body_mode: BodyMode,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_file: Option<PathBuf>,
    #[serde(default)]
    pub output_json: Option<PathBuf>,
}

impl Default for BenchmarkConfigInput {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: default_method(),
            concurrency: default_concurrency(),
            duration: default_duration_str(),
            ramp_up: default_ramp_up_str(),
            timeout: default_timeout_str(),
            headers: Vec::new(),
            body_mode: BodyMode::None,
            body: None,
            body_file: None,
            output_json: None,
        }
    }
}

/// Structured validation result returned to interactive UI clients before starting a run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub field_errors: HashMap<String, String>,
    pub warnings: Vec<String>,
}

/// Validates a [`BenchmarkConfigInput`] and returns per-field errors plus OS limit warnings.
pub fn validate_input(
    input: &BenchmarkConfigInput,
    os_limits: Option<OsLimits>,
) -> ValidationReport {
    let mut field_errors = HashMap::new();
    let mut warnings = Vec::new();

    if let Err(err) = validate_target_url(&input.url) {
        field_errors.insert("url".to_string(), err.to_string());
    }

    if input.concurrency == 0 {
        field_errors.insert(
            "concurrency".to_string(),
            ConfigError::InvalidConcurrency(input.concurrency).to_string(),
        );
    }

    let parsed_duration = match parse_duration(&input.duration) {
        Ok(d) if d.is_zero() => {
            field_errors.insert("duration".to_string(), ConfigError::InvalidDuration.to_string());
            None
        }
        Ok(d) => Some(d),
        Err(reason) => {
            field_errors.insert("duration".to_string(), reason);
            None
        }
    };

    match parse_timeout(&input.timeout) {
        Ok(t) if t.is_zero() => {
            field_errors.insert("timeout".to_string(), ConfigError::InvalidTimeout.to_string());
        }
        Ok(_) => {}
        Err(reason) => {
            field_errors.insert("timeout".to_string(), reason);
        }
    }

    match parse_duration(&input.ramp_up) {
        Ok(ramp_up) => {
            if let Some(duration) = parsed_duration {
                if ramp_up > duration {
                    field_errors.insert(
                        "ramp_up".to_string(),
                        ConfigError::RampUpExceedsDuration { ramp_up, duration }.to_string(),
                    );
                }
            }
        }
        Err(reason) => {
            field_errors.insert("ramp_up".to_string(), reason);
        }
    }

    for (idx, entry) in input.headers.iter().enumerate() {
        if !entry.enabled {
            continue;
        }
        if let Err(err) = validate_header_entry(&entry.name, &entry.value) {
            field_errors.insert(format!("headers.{idx}"), err.to_string());
        }
    }

    if input.body_mode == BodyMode::File {
        match input.body_file.as_ref() {
            None => {
                field_errors.insert(
                    "body_file".to_string(),
                    "select a request payload file".to_string(),
                );
            }
            Some(path) => {
                let path_str = path.to_string_lossy();
                let trimmed = path_str.trim();
                if trimmed.is_empty() {
                    field_errors.insert(
                        "body_file".to_string(),
                        "request body file path cannot be empty".to_string(),
                    );
                } else {
                    match fs::metadata(trimmed) {
                        Ok(meta) if meta.is_dir() => {
                            field_errors.insert(
                                "body_file".to_string(),
                                format!("'{trimmed}' is a directory, expected a file"),
                            );
                        }
                        Ok(_) => {
                            if let Err(err) = fs::File::open(trimmed) {
                                field_errors.insert(
                                    "body_file".to_string(),
                                    format!("cannot open '{trimmed}': {err}"),
                                );
                            }
                        }
                        Err(err) => {
                            field_errors.insert(
                                "body_file".to_string(),
                                format!("cannot access '{trimmed}': {err}"),
                            );
                        }
                    }
                }
            }
        }
    }

    if let Some(limits) = os_limits {
        let estimated_fds = (input.concurrency as u64).saturating_mul(2).saturating_add(128);
        if !limits.is_sufficient() || estimated_fds > limits.soft_limit {
            warnings.push(format!(
                "OS open file limit is {} (concurrency {} needs ~{} descriptors, recommended >= {}). Run 'ulimit -n 65536' if running from a terminal.",
                limits.soft_limit,
                input.concurrency,
                estimated_fds,
                limits.recommended_min
            ));
        }
    }

    ValidationReport {
        valid: field_errors.is_empty(),
        field_errors,
        warnings,
    }
}

fn validate_target_url(raw_url: &str) -> Result<Url, ConfigError> {
    let trimmed = raw_url.trim();
    let parsed_url = Url::parse(trimmed).map_err(|e| ConfigError::InvalidUrl {
        url: raw_url.to_string(),
        reason: e.to_string(),
    })?;

    let scheme = parsed_url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(ConfigError::UnsupportedScheme {
            scheme: scheme.to_string(),
        });
    }

    if parsed_url.host_str().is_none() {
        return Err(ConfigError::InvalidUrl {
            url: raw_url.to_string(),
            reason: "missing host".to_string(),
        });
    }

    Ok(parsed_url)
}

fn validate_header_entry(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), ConfigError> {
    let name_trimmed = name.trim();
    if name_trimmed.is_empty() {
        return Err(ConfigError::InvalidHeader(
            "header name cannot be empty".to_string(),
        ));
    }

    let header_name = HeaderName::from_str(name_trimmed).map_err(|e| {
        ConfigError::InvalidHeader(format!("invalid header name '{name_trimmed}': {e}"))
    })?;

    let value_trimmed = value.trim();
    let header_value = HeaderValue::from_str(value_trimmed).map_err(|e| {
        ConfigError::InvalidHeader(format!("invalid header value for '{name_trimmed}': {e}"))
    })?;

    Ok((header_name, header_value))
}

/// Returns `true` if the HTTP header name likely carries credentials, tokens, or secrets.
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "api-key"
            | "x-auth-token"
    ) || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("credential")
        || lower.contains("key")
}

/// Redacts embedded userinfo (`username:password@`) from a URL string when valid.
pub fn sanitize_url(url: &Url) -> String {
    let mut sanitized = url.clone();
    if !sanitized.username().is_empty() {
        let _ = sanitized.set_username("REDACTED");
    }
    if sanitized.password().is_some() {
        let _ = sanitized.set_password(Some("REDACTED"));
    }
    sanitized.to_string()
}

fn read_body_file(raw_path: &str) -> Result<String, ConfigError> {
    let trimmed_path = raw_path.trim();
    if trimmed_path.is_empty() {
        return Err(ConfigError::EmptyBodyFilePath);
    }
    fs::read_to_string(trimmed_path).map_err(|source| ConfigError::BodyFileReadError {
        path: trimmed_path.to_string(),
        source,
    })
}

/// Validated configuration for the embedded real-time GUI monitoring server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiConfig {
    pub enabled: bool,
    pub port: u16,
    pub bind_addr: IpAddr,
    pub open_browser: bool,
    pub keep_alive: bool,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8080,
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            open_browser: false,
            keep_alive: false,
        }
    }
}

impl GuiConfig {
    /// Validates and constructs [`GuiConfig`] from parsed [`Cli`] arguments.
    pub fn from_cli(cli: &Cli) -> Result<Self, ConfigError> {
        let bind_addr = cli.gui_bind.trim().parse::<IpAddr>().map_err(|e| {
            ConfigError::InvalidGuiBindAddress {
                addr: cli.gui_bind.clone(),
                reason: e.to_string(),
            }
        })?;

        Ok(Self {
            enabled: cli.gui,
            port: cli.gui_port,
            bind_addr,
            open_browser: cli.gui_open,
            keep_alive: cli.gui_keep_alive,
        })
    }

    /// Returns the resolved [`SocketAddr`] (`bind_addr:port`) for the GUI HTTP server.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_addr, self.port)
    }
}

/// Strongly typed, validated runtime configuration for a benchmark run.
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub url: Url,
    pub concurrency: usize,
    pub duration: Duration,
    pub method: Method,
    pub headers: HeaderMap,
    pub body: Option<String>,
    pub timeout: Duration,
    pub ramp_up: Duration,
    pub output_json: Option<PathBuf>,
}

impl BenchmarkConfig {
    fn build_from_parts(
        raw_url: &str,
        concurrency: usize,
        duration: Duration,
        timeout: Duration,
        ramp_up: Duration,
        method: HttpMethod,
        headers: HeaderMap,
        body: Option<String>,
        output_json: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let parsed_url = validate_target_url(raw_url)?;

        if concurrency == 0 {
            return Err(ConfigError::InvalidConcurrency(concurrency));
        }

        if duration.is_zero() {
            return Err(ConfigError::InvalidDuration);
        }

        if timeout.is_zero() {
            return Err(ConfigError::InvalidTimeout);
        }

        if ramp_up > duration {
            return Err(ConfigError::RampUpExceedsDuration { ramp_up, duration });
        }

        Ok(Self {
            url: parsed_url,
            concurrency,
            duration,
            method: Method::from(method),
            headers,
            body,
            timeout,
            ramp_up,
            output_json,
        })
    }

    /// Validates and converts parsed [`Cli`] arguments into a [`BenchmarkConfig`].
    pub fn from_cli(cli: Cli) -> Result<Self, ConfigError> {
        let _ = GuiConfig::from_cli(&cli)?;

        let mut headers = HeaderMap::new();
        for raw_header in &cli.headers {
            let (name, value) =
                parse_header(raw_header).map_err(ConfigError::InvalidHeader)?;
            headers.append(name, value);
        }

        let body = match cli.body {
            Some(raw_body) => {
                if let Some(file_path) = raw_body.strip_prefix('@') {
                    Some(read_body_file(file_path)?)
                } else {
                    Some(raw_body)
                }
            }
            None => None,
        };

        Self::build_from_parts(
            &cli.url,
            cli.concurrency,
            cli.duration,
            cli.timeout,
            cli.ramp_up,
            cli.method,
            headers,
            body,
            cli.output_json,
        )
    }

    /// Validates and converts an interactive [`BenchmarkConfigInput`] into a [`BenchmarkConfig`].
    pub fn from_input(input: BenchmarkConfigInput) -> Result<Self, ConfigError> {
        let duration =
            parse_duration(&input.duration).map_err(ConfigError::InvalidDurationFormat)?;
        let timeout = parse_timeout(&input.timeout).map_err(ConfigError::InvalidTimeoutFormat)?;
        let ramp_up =
            parse_duration(&input.ramp_up).map_err(ConfigError::InvalidRampUpFormat)?;

        let mut headers = HeaderMap::new();
        for entry in &input.headers {
            if !entry.enabled {
                continue;
            }
            let (name, value) = validate_header_entry(&entry.name, &entry.value)?;
            headers.append(name, value);
        }

        let body = match input.body_mode {
            BodyMode::None => None,
            BodyMode::Raw => input.body.filter(|raw| !raw.is_empty()),
            BodyMode::File => {
                let path = input
                    .body_file
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .ok_or(ConfigError::EmptyBodyFilePath)?;
                Some(read_body_file(&path)?)
            }
        };

        Self::build_from_parts(
            &input.url,
            input.concurrency,
            duration,
            timeout,
            ramp_up,
            input.method,
            headers,
            body,
            input.output_json,
        )
    }

    /// Returns the resolved request body as a byte slice, if configured.
    pub fn body_bytes(&self) -> Option<&[u8]> {
        self.body.as_deref().map(str::as_bytes)
    }
}

impl TryFrom<Cli> for BenchmarkConfig {
    type Error = ConfigError;

    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        Self::from_cli(cli)
    }
}

impl Cli {
    /// Converts this [`Cli`] instance into a validated [`BenchmarkConfig`].
    pub fn into_config(self) -> Result<BenchmarkConfig, ConfigError> {
        BenchmarkConfig::from_cli(self)
    }

    /// Extracts and validates the [`GuiConfig`] from this [`Cli`] instance.
    pub fn gui_config(&self) -> Result<GuiConfig, ConfigError> {
        GuiConfig::from_cli(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_validate_input_catches_invalid_fields_and_fd_warnings() {
        let input = BenchmarkConfigInput {
            url: "ftp://bad.example.com".to_string(),
            method: HttpMethod::Post,
            concurrency: 500,
            duration: "10s".to_string(),
            ramp_up: "20s".to_string(),
            timeout: "0ms".to_string(),
            headers: vec![HeaderEntry {
                name: "".to_string(),
                value: "abc".to_string(),
                enabled: true,
            }],
            body_mode: BodyMode::File,
            body: None,
            body_file: None,
            output_json: None,
        };

        let limits = OsLimits {
            soft_limit: 256,
            hard_limit: 65_536,
            recommended_min: 4096,
        };
        let report = validate_input(&input, Some(limits));
        assert!(!report.valid);
        assert!(report.field_errors.contains_key("url"));
        assert!(report.field_errors.contains_key("ramp_up"));
        assert!(report.field_errors.contains_key("timeout"));
        assert!(report.field_errors.contains_key("headers.0"));
        assert!(report.field_errors.contains_key("body_file"));
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn test_benchmark_config_from_input_file_body() {
        let mut tmp = NamedTempFile::new().expect("temp file");
        write!(tmp, r#"{{"ok":true}}"#).expect("write payload");

        let input = BenchmarkConfigInput {
            url: "https://api.example.com/v1/items".to_string(),
            method: HttpMethod::Post,
            concurrency: 32,
            duration: "15s".to_string(),
            ramp_up: "3s".to_string(),
            timeout: "1500ms".to_string(),
            headers: vec![
                HeaderEntry {
                    name: "Content-Type".to_string(),
                    value: "application/json".to_string(),
                    enabled: true,
                },
                HeaderEntry {
                    name: "X-Disabled".to_string(),
                    value: "ignored".to_string(),
                    enabled: false,
                },
            ],
            body_mode: BodyMode::File,
            body: None,
            body_file: Some(tmp.path().to_path_buf()),
            output_json: None,
        };

        let cfg = BenchmarkConfig::from_input(input).expect("valid input");
        assert_eq!(cfg.method, Method::POST);
        assert_eq!(cfg.concurrency, 32);
        assert_eq!(cfg.duration, Duration::from_secs(15));
        assert_eq!(cfg.ramp_up, Duration::from_secs(3));
        assert_eq!(cfg.timeout, Duration::from_millis(1500));
        assert_eq!(cfg.headers.len(), 1);
        assert_eq!(cfg.body.as_deref(), Some(r#"{"ok":true}"#));
    }
}
