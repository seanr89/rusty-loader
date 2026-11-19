use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use thiserror::Error;

use crate::cli::{parse_header, Cli};

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

    #[error("request timeout must be greater than 0ms")]
    InvalidTimeout,

    #[error("ramp-up duration ({ramp_up:?}) cannot exceed total benchmark duration ({duration:?})")]
    RampUpExceedsDuration {
        ramp_up: Duration,
        duration: Duration,
    },

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
    /// Validates and converts parsed [`Cli`] arguments into a [`BenchmarkConfig`].
    pub fn from_cli(cli: Cli) -> Result<Self, ConfigError> {
        let parsed_url = Url::parse(&cli.url).map_err(|e| ConfigError::InvalidUrl {
            url: cli.url.clone(),
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
                url: cli.url,
                reason: "missing host".to_string(),
            });
        }

        if cli.concurrency == 0 {
            return Err(ConfigError::InvalidConcurrency(cli.concurrency));
        }

        if cli.duration.is_zero() {
            return Err(ConfigError::InvalidDuration);
        }

        if cli.timeout.is_zero() {
            return Err(ConfigError::InvalidTimeout);
        }

        if cli.ramp_up > cli.duration {
            return Err(ConfigError::RampUpExceedsDuration {
                ramp_up: cli.ramp_up,
                duration: cli.duration,
            });
        }

        let mut headers = HeaderMap::new();
        for raw_header in &cli.headers {
            let (name, value) =
                parse_header(raw_header).map_err(ConfigError::InvalidHeader)?;
            headers.append(name, value);
        }

        let body = match cli.body {
            Some(raw_body) => {
                if let Some(file_path) = raw_body.strip_prefix('@') {
                    let trimmed_path = file_path.trim();
                    if trimmed_path.is_empty() {
                        return Err(ConfigError::EmptyBodyFilePath);
                    }
                    let contents = fs::read_to_string(trimmed_path).map_err(|source| {
                        ConfigError::BodyFileReadError {
                            path: trimmed_path.to_string(),
                            source,
                        }
                    })?;
                    Some(contents)
                } else {
                    Some(raw_body)
                }
            }
            None => None,
        };

        Ok(Self {
            url: parsed_url,
            concurrency: cli.concurrency,
            duration: cli.duration,
            method: Method::from(cli.method),
            headers,
            body,
            timeout: cli.timeout,
            ramp_up: cli.ramp_up,
            output_json: cli.output_json,
        })
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
}
