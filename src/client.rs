//! HTTP client builder and connection pooling configuration.

use std::time::Duration;

use anyhow::Result;
use reqwest::redirect::Policy;
use reqwest::Client;

use crate::config::BenchmarkConfig;

/// Default TCP keepalive duration for persistent pooled connections.
pub const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(60);

/// Default User-Agent header sent by rusty-loader.
pub const DEFAULT_USER_AGENT: &str = "rusty-loader/0.1.0";

/// Factory for constructing a high-concurrency [`reqwest::Client`] tuned for load testing.
pub struct HttpClientFactory;

impl HttpClientFactory {
    /// Builds a [`reqwest::Client`] configured for persistent connection pooling and low latency.
    pub fn build(config: &BenchmarkConfig) -> Result<Client> {
        build_client(config)
    }
}

/// Builds a [`reqwest::Client`] tuned for `config.concurrency` persistent connections.
///
/// Configuration applied:
/// - `pool_max_idle_per_host(config.concurrency)`
/// - `tcp_nodelay(true)`
/// - `tcp_keepalive(Some(Duration::from_secs(60)))`
/// - `timeout(config.timeout)`
/// - `redirect(Policy::none())`
/// - `user_agent("rusty-loader/0.1.0")`
/// - `default_headers(config.headers.clone())`
pub fn build_client(config: &BenchmarkConfig) -> Result<Client> {
    Client::builder()
        .pool_max_idle_per_host(config.concurrency)
        .tcp_nodelay(true)
        .tcp_keepalive(Some(DEFAULT_TCP_KEEPALIVE))
        .timeout(config.timeout)
        .redirect(Policy::none())
        .no_proxy()
        .user_agent(DEFAULT_USER_AGENT)
        .default_headers(config.headers.clone())
        .build()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    use reqwest::{Method, Url};

    fn sample_config(concurrency: usize, timeout: Duration) -> BenchmarkConfig {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer test-token"));

        BenchmarkConfig {
            url: Url::parse("http://127.0.0.1:8080/health").expect("valid url"),
            concurrency,
            duration: Duration::from_secs(10),
            method: Method::GET,
            headers,
            body: None,
            timeout,
            ramp_up: Duration::ZERO,
            output_json: None,
        }
    }

    #[test]
    fn test_build_client_succeeds_for_high_concurrency() {
        let config = sample_config(700, Duration::from_secs(5));
        let client = build_client(&config);
        assert!(client.is_ok(), "build_client should succeed for 700 concurrency");
    }

    #[test]
    fn test_http_client_factory_build() {
        let config = sample_config(128, Duration::from_millis(2500));
        let client = HttpClientFactory::build(&config);
        assert!(client.is_ok(), "HttpClientFactory::build should succeed");
    }
}
