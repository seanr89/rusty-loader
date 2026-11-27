use reqwest::StatusCode;
use std::time::Duration;

/// Specific non-transport check failure recorded for a step in an ordered API sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepFailureKind {
    AssertionFailed,
    ExtractionFailed,
}

/// End-of-iteration outcome recorded for ordered API sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JourneyOutcome {
    Completed {
        duration: Duration,
    },
    Aborted {
        failed_step_index: u16,
        duration: Duration,
    },
}

/// Notification emitted by an HTTP worker after each request attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricEvent {
    /// End-to-end elapsed latency for the request attempt.
    pub latency: Duration,
    /// HTTP status code if a response was received.
    pub status_code: Option<StatusCode>,
    /// Number of response body bytes received.
    pub bytes_received: usize,
    /// Optional transport, assertion, or extraction error description when the attempt fails.
    pub error: Option<String>,
    /// Optional 0-based compiled step index when executing an ordered API sequence.
    pub step_index: Option<u16>,
    /// Optional classification when an HTTP response failed step assertions or extractors.
    pub failure_kind: Option<StepFailureKind>,
    /// Optional journey-level completion or abort summary emitted at iteration end.
    pub journey_outcome: Option<JourneyOutcome>,
}

impl MetricEvent {
    /// Creates a `MetricEvent` representing a completed HTTP response.
    pub fn response(latency: Duration, status_code: StatusCode, bytes_received: usize) -> Self {
        Self {
            latency,
            status_code: Some(status_code),
            bytes_received,
            error: None,
            step_index: None,
            failure_kind: None,
            journey_outcome: None,
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
            step_index: None,
            failure_kind: None,
            journey_outcome: None,
        }
    }

    /// Attaches the 0-based compiled sequence step index to this event.
    pub fn with_step_index(mut self, step_index: u16) -> Self {
        self.step_index = Some(step_index);
        self
    }

    /// Marks an HTTP response event as having failed a step assertion or variable extraction.
    pub fn with_failure_kind(mut self, kind: StepFailureKind, reason: impl Into<String>) -> Self {
        self.failure_kind = Some(kind);
        self.error = Some(reason.into());
        self
    }

    /// Attaches an iteration journey outcome (`Completed` or `Aborted`) to this event.
    pub fn with_journey_outcome(mut self, outcome: JourneyOutcome) -> Self {
        self.journey_outcome = Some(outcome);
        self
    }
}
