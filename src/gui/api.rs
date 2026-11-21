//! Axum HTTP handlers, SSE telemetry streaming, and metadata redaction for the GUI server.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::changelog::{about_payload, changelog_payload, AboutPayload, ChangelogPayload};
use crate::config::BenchmarkConfig;
use crate::metrics::snapshot::{TelemetryHub, TelemetryPhase, TelemetrySnapshot};
use crate::metrics::stats::{BenchmarkStats, LatencyDistribution, LiveCounters};
use crate::reporter::JsonReport;

const INDEX_HTML: &str = include_str!("assets/index.html");
const STYLES_CSS: &str = include_str!("assets/styles.css");
const APP_JS: &str = include_str!("assets/app.js");

const REDACTED_VALUE: &str = "[REDACTED]";

/// Shared Axum application state for all GUI endpoints.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<BenchmarkConfig>,
    pub hub: Arc<TelemetryHub>,
    pub counters: Arc<LiveCounters>,
    pub cancel_token: CancellationToken,
    pub started_at: Instant,
}

/// Sanitized benchmark metadata returned by `GET /api/meta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaResponse {
    pub url: String,
    pub method: String,
    pub concurrency: usize,
    pub duration_secs: f64,
    pub ramp_up_secs: f64,
    pub timeout_ms: u64,
    pub headers: HashMap<String, String>,
    pub has_body: bool,
    pub body_size_bytes: usize,
    pub phase: TelemetryPhase,
}

/// Builds the Axum [`Router`] for the embedded dashboard UI and telemetry/control endpoints.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/styles.css", get(serve_styles))
        .route("/assets/app.js", get(serve_app_js))
        .route("/api/meta", get(get_meta))
        .route("/api/about", get(get_about))
        .route("/api/changelog", get(get_changelog))
        .route("/api/history", get(get_history))
        .route("/api/stream", get(stream_telemetry))
        .route("/api/stop", post(post_stop))
        .route("/api/export/json", get(export_json))
        .with_state(state)
}

/// Returns `true` if the HTTP header name likely carries credentials, tokens, or secrets.
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
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

/// Clones `headers` into a map while replacing sensitive header values with `"[REDACTED]"`.
pub fn redact_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let mut sanitized = HashMap::new();
    for (name, value) in headers {
        let key = name.as_str().to_string();
        let val = if is_sensitive_header(name.as_str()) {
            REDACTED_VALUE.to_string()
        } else {
            value
                .to_str()
                .map(str::to_string)
                .unwrap_or_else(|_| "[BINARY]".to_string())
        };
        sanitized.insert(key, val);
    }
    sanitized
}

/// Redacts embedded userinfo (`username:password@`) from a URL before exposing it in GUI metadata.
pub fn sanitize_url(url: &reqwest::Url) -> String {
    let mut sanitized = url.clone();
    if !sanitized.username().is_empty() {
        let _ = sanitized.set_username("REDACTED");
    }
    if sanitized.password().is_some() {
        let _ = sanitized.set_password(Some("REDACTED"));
    }
    sanitized.to_string()
}

async fn serve_index() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        INDEX_HTML,
    )
}

async fn serve_styles() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        STYLES_CSS,
    )
}

async fn serve_app_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        APP_JS,
    )
}

async fn get_meta(State(state): State<AppState>) -> Json<MetaResponse> {
    let phase = state.hub.current_phase(state.started_at.elapsed());

    Json(MetaResponse {
        url: sanitize_url(&state.config.url),
        method: state.config.method.as_str().to_string(),
        concurrency: state.config.concurrency,
        duration_secs: state.config.duration.as_secs_f64(),
        ramp_up_secs: state.config.ramp_up.as_secs_f64(),
        timeout_ms: state.config.timeout.as_millis() as u64,
        headers: redact_headers(&state.config.headers),
        has_body: state.config.body.is_some(),
        body_size_bytes: state.config.body_bytes().map(<[u8]>::len).unwrap_or(0),
        phase,
    })
}

async fn get_about() -> Json<AboutPayload> {
    Json(about_payload())
}

async fn get_changelog() -> Json<ChangelogPayload> {
    Json(changelog_payload())
}

async fn get_history(State(state): State<AppState>) -> Json<Vec<TelemetrySnapshot>> {
    let snapshots = state
        .hub
        .history()
        .into_iter()
        .map(|s| (*s).clone())
        .collect();
    Json(snapshots)
}

async fn post_stop(State(state): State<AppState>) -> Json<serde_json::Value> {
    state.hub.mark_draining();
    state.cancel_token.cancel();
    Json(serde_json::json!({
        "status": "stopping",
        "cancelled": true,
    }))
}

async fn export_json(State(state): State<AppState>) -> Response {
    let stats = match state.hub.final_stats() {
        Some(final_stats) => (*final_stats).clone(),
        None => build_partial_stats(&state),
    };

    let mut report = JsonReport::new(&state.config, &stats);
    report.url = sanitize_url(&state.config.url);
    report.headers = redact_headers(&state.config.headers);

    match serde_json::to_string_pretty(&report) {
        Ok(payload) => {
            let mut response = (StatusCode::OK, payload).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            );
            response.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"rusty-loader-report.json\""),
            );
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
            response
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize benchmark report: {err}"),
        )
            .into_response(),
    }
}

fn build_partial_stats(state: &AppState) -> BenchmarkStats {
    if let Some(latest) = state.hub.latest_snapshot() {
        let lat = &latest.window_latency_ms;
        return BenchmarkStats {
            total_duration_secs: latest.elapsed_secs,
            total_requests: latest.total_requests,
            successful_requests: latest.successful_requests,
            client_errors: latest.client_errors,
            server_errors: latest.server_errors,
            other_statuses: latest.other_statuses,
            transport_errors: latest.transport_errors,
            requests_per_sec: latest.avg_rps,
            total_bytes: latest.total_bytes,
            bytes_per_sec: if latest.elapsed_secs > 0.0 {
                latest.total_bytes as f64 / latest.elapsed_secs
            } else {
                0.0
            },
            latencies: LatencyDistribution {
                min_ms: lat.min,
                mean_ms: lat.mean,
                p50_ms: lat.p50,
                p90_ms: lat.p90,
                p95_ms: lat.p95,
                p99_ms: lat.p99,
                p99_9_ms: lat.p99,
                max_ms: lat.max,
                std_dev_ms: 0.0,
            },
            status_distribution: latest.status_distribution.clone(),
            error_breakdown: latest.error_breakdown.clone(),
        };
    }

    let elapsed = state.started_at.elapsed().as_secs_f64();
    let total_requests = state.counters.total_requests();
    let total_bytes = state.counters.total_bytes();

    BenchmarkStats {
        total_duration_secs: elapsed,
        total_requests,
        successful_requests: state.counters.successful_requests(),
        client_errors: state.counters.client_errors(),
        server_errors: state.counters.server_errors(),
        other_statuses: state.counters.other_statuses(),
        transport_errors: state.counters.transport_errors(),
        requests_per_sec: if elapsed > 0.0 {
            total_requests as f64 / elapsed
        } else {
            0.0
        },
        total_bytes,
        bytes_per_sec: if elapsed > 0.0 {
            total_bytes as f64 / elapsed
        } else {
            0.0
        },
        latencies: LatencyDistribution::default(),
        status_distribution: HashMap::new(),
        error_breakdown: HashMap::new(),
    }
}

struct SseStreamState {
    broadcast_rx: tokio::sync::broadcast::Receiver<Arc<TelemetrySnapshot>>,
    completion_rx: tokio::sync::watch::Receiver<Option<Arc<BenchmarkStats>>>,
    pending: VecDeque<Event>,
    done: bool,
}

fn snapshot_to_event(snapshot: &TelemetrySnapshot) -> Option<Event> {
    Event::default()
        .event("snapshot")
        .json_data(snapshot)
        .ok()
}

fn complete_to_event(stats: &BenchmarkStats) -> Option<Event> {
    Event::default().event("complete").json_data(stats).ok()
}

async fn stream_telemetry(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let initial = SseStreamState {
        broadcast_rx: state.hub.subscribe(),
        completion_rx: state.hub.subscribe_completion(),
        pending: VecDeque::new(),
        done: false,
    };

    let event_stream = stream::unfold(initial, |mut s| async move {
        loop {
            if let Some(ev) = s.pending.pop_front() {
                return Some((Ok(ev), s));
            }
            if s.done {
                return None;
            }

            if let Some(stats) = s.completion_rx.borrow().clone() {
                while let Ok(snap) = s.broadcast_rx.try_recv() {
                    if let Some(ev) = snapshot_to_event(&snap) {
                        s.pending.push_back(ev);
                    }
                }
                if let Some(ev) = complete_to_event(&stats) {
                    s.pending.push_back(ev);
                }
                s.done = true;
                continue;
            }

            tokio::select! {
                biased;
                res = s.broadcast_rx.recv() => {
                    match res {
                        Ok(snap) => {
                            if let Some(ev) = snapshot_to_event(&snap) {
                                return Some((Ok(ev), s));
                            }
                        }
                        Err(RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(RecvError::Closed) => {
                            if let Some(stats) = s.completion_rx.borrow().clone() {
                                if let Some(ev) = complete_to_event(&stats) {
                                    s.pending.push_back(ev);
                                }
                            }
                            s.done = true;
                        }
                    }
                }
                changed = s.completion_rx.changed() => {
                    match changed {
                        Ok(()) => {
                            if let Some(stats) = s.completion_rx.borrow().clone() {
                                while let Ok(snap) = s.broadcast_rx.try_recv() {
                                    if let Some(ev) = snapshot_to_event(&snap) {
                                        s.pending.push_back(ev);
                                    }
                                }
                                if let Some(ev) = complete_to_event(&stats) {
                                    s.pending.push_back(ev);
                                }
                                s.done = true;
                            }
                        }
                        Err(_) => {
                            s.done = true;
                        }
                    }
                }
            }
        }
    });

    Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
