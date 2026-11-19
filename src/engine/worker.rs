//! Single worker task loop executing HTTP requests and sampling high-resolution timings.

use std::sync::Arc;
use std::time::Instant;

use reqwest::{Body, Client, Request};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::config::BenchmarkConfig;
use crate::metrics::event::MetricEvent;

/// Asynchronous load-generation worker that continuously sends HTTP requests
/// using a shared [`reqwest::Client`] until cancelled.
#[derive(Debug, Clone)]
pub struct Worker {
    pub id: usize,
    pub client: Client,
    pub config: Arc<BenchmarkConfig>,
    pub sender: UnboundedSender<MetricEvent>,
    pub cancel_token: CancellationToken,
}

impl Worker {
    /// Creates a new [`Worker`] instance.
    pub fn new(
        id: usize,
        client: Client,
        config: Arc<BenchmarkConfig>,
        sender: UnboundedSender<MetricEvent>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            id,
            client,
            config,
            sender,
            cancel_token,
        }
    }

    /// Returns the worker's numeric identifier.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Constructs a fresh [`reqwest::Request`] from the shared [`BenchmarkConfig`].
    pub fn build_request(&self) -> Request {
        let mut request = Request::new(self.config.method.clone(), self.config.url.clone());

        if !self.config.headers.is_empty() {
            *request.headers_mut() = self.config.headers.clone();
        }

        if let Some(body) = &self.config.body {
            *request.body_mut() = Some(Body::from(body.clone()));
        }

        request
    }

    /// Executes a single HTTP request attempt, draining the full response body
    /// before computing end-to-end latency.
    pub async fn execute_once(&self) -> MetricEvent {
        let request = self.build_request();
        let start = Instant::now();

        match self.client.execute(request).await {
            Ok(response) => {
                let status = response.status();
                match response.bytes().await {
                    Ok(body_bytes) => {
                        let latency = start.elapsed();
                        let len = body_bytes.len();
                        drop(body_bytes);
                        MetricEvent::response(latency, status, len)
                    }
                    Err(err) => {
                        let latency = start.elapsed();
                        MetricEvent::error(latency, classify_reqwest_error(&err))
                    }
                }
            }
            Err(err) => {
                let latency = start.elapsed();
                MetricEvent::error(latency, classify_reqwest_error(&err))
            }
        }
    }

    /// Runs the continuous load generation loop until `cancel_token` is cancelled
    /// or the metrics receiver channel closes.
    pub async fn run(self) {
        loop {
            if self.cancel_token.is_cancelled() {
                break;
            }

            tokio::select! {
                biased;
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                event = self.execute_once() => {
                    if self.sender.send(event).is_err() {
                        break;
                    }
                }
            }

            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        }
    }
}

/// Classifies a [`reqwest::Error`] into a concise, deterministic category label.
pub fn classify_reqwest_error(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "Timeout"
    } else if err.is_connect() {
        "ConnectError"
    } else if err.is_body() || err.is_decode() {
        "BodyReadError"
    } else if err.is_redirect() {
        "RedirectError"
    } else if err.is_request() {
        "RequestError"
    } else if err.is_builder() {
        "BuilderError"
    } else {
        "NetworkError"
    }
}
