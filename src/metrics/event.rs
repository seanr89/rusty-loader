use reqwest::StatusCode;
use std::time::Duration;

/// Notification emitted by an HTTP worker after each request attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricEvent {
    /// End-to-end elapsed latency for the request attempt.
    pub latency: Duration,
    /// HTTP status code if a response was received.
    pub status_code: Option<StatusCode>,
    /// Number of response body bytes received.
    pub bytes_received: usize,
    /// Optional transport or request error description when the request fails.
    pub error: Option<String>,
}

impl MetricEvent {
    /// Creates a `MetricEvent` representing a completed HTTP response.
    pub fn response(latency: Duration, status_code: StatusCode, bytes_received: usize) -> Self {
        Self {
            latency,
            status_code: Some(status_code),
            bytes_received,
            error: None,
        }
    }

    /// Alias for [`MetricEvent::response`].
    pub fn success(latency: Duration, status_code: StatusCode, bytes_received: usize) -> Self {
        Self::response(latency, status_code, bytes_received)
    }

    /// Creates a `MetricEvent` representing a transport or network error (e.g., timeout, connect error).
    pub fn error(latency: Duration, error: impl Into<String>) -> Self {
        Self {
            latency,
            status_code: None,
            bytes_received: 0,
            error: Some(error.into()),
        }
    }
}
