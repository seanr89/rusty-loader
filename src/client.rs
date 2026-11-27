//! HTTP client builder and connection pooling configuration.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Result;
use reqwest::redirect::Policy;
use reqwest::Client;
use tower_layer::Layer;
use tower_service::Service;

use crate::config::BenchmarkConfig;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Default TCP keepalive duration for persistent pooled connections.
pub const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(60);

/// Default User-Agent header sent by rusty-loader.
pub const DEFAULT_USER_AGENT: &str = "rusty-loader/0.1.0";

/// Connector layer that throttles and cancels speculative background connects
/// spawned by `hyper-util` once `concurrency` connections have been established.
#[derive(Clone)]
struct PoolCapConnectorLayer {
    concurrency: usize,
    established: Arc<AtomicUsize>,
}

impl PoolCapConnectorLayer {
    fn new(concurrency: usize) -> Self {
        Self {
            concurrency,
            established: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl<S> Layer<S> for PoolCapConnectorLayer {
    type Service = PoolCapConnectorService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PoolCapConnectorService {
            inner,
            concurrency: self.concurrency,
            established: Arc::clone(&self.established),
        }
    }
}

#[derive(Clone)]
struct PoolCapConnectorService<S> {
    inner: S,
    concurrency: usize,
    established: Arc<AtomicUsize>,
}

impl<S, Req> Service<Req> for PoolCapConnectorService<S>
where
    S: Service<Req> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Into<BoxError>,
    Req: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        let concurrency = self.concurrency;
        let established = Arc::clone(&self.established);

        Box::pin(async move {
            let caller_task_id = tokio::task::try_id();

            if established.load(Ordering::Relaxed) >= concurrency {
                tokio::time::sleep(Duration::from_millis(2)).await;
                if caller_task_id.is_some() && tokio::task::try_id() != caller_task_id {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "checkout won; cancelling speculative background connect",
                    )) as BoxError);
                }
            }

            let conn = inner.call(req).await.map_err(Into::into)?;

            if caller_task_id.is_some() && tokio::task::try_id() != caller_task_id {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "checkout won; cancelling speculative background connect",
                )) as BoxError);
            }

            established.fetch_add(1, Ordering::Relaxed);
            Ok(conn)
        })
    }
}

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
        .connector_layer(PoolCapConnectorLayer::new(config.concurrency))
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
            output_csv: None,
            output_pdf: None,
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
