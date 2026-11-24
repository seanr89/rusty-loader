//! Interactive desktop/web workbench HTTP handlers, session authentication, and multi-run SSE.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;

use crate::changelog::{about_payload, changelog_payload, APP_VERSION};
use crate::config::{validate_input, BenchmarkConfigInput, ValidationReport};
use crate::gui::api::{
    serve_app_js, serve_index, serve_styles, GuiMode, MetaResponse,
};
use crate::gui::manager::{RunLifecycleEvent, RunManager, RunManagerError};
use crate::metrics::snapshot::TelemetrySnapshot;
use crate::os::OsLimits;
use crate::presets::{format_cli_command, PresetStore, PresetUpsertRequest};

const SESSION_HEADER: &str = "x-rl-session";
const SESSION_COOKIE_NAME: &str = "rl_session";

/// Request routed from Axum handlers to the macOS main thread for native `NSOpenPanel`/`NSSavePanel` UI.
pub enum NativeDialogRequest {
    OpenBodyFile {
        resp: oneshot::Sender<Option<PathBuf>>,
    },
    SaveJsonFile {
        default_name: String,
        resp: oneshot::Sender<Option<PathBuf>>,
    },
}

/// Callback dispatching [`NativeDialogRequest`] onto the native UI event loop.
pub type DialogDispatcher = Arc<dyn Fn(NativeDialogRequest) -> bool + Send + Sync>;

/// Shared Axum state for the interactive desktop and `--ui-serve` workbench.
#[derive(Clone)]
pub struct InteractiveState {
    pub manager: Arc<RunManager>,
    pub presets: Arc<PresetStore>,
    pub session_token: Arc<String>,
    pub dialog_dispatcher: Option<DialogDispatcher>,
    pub initial_config: Option<BenchmarkConfigInput>,
}

/// Generates a 256-bit cryptographically random hex session token.
pub fn generate_session_token() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|err| anyhow::anyhow!("failed to generate random session token: {err}"))?;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(hex)
}

#[derive(Debug, Deserialize)]
struct BootstrapQuery {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RunIdQuery {
    run_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct SystemInfoResponse {
    os: &'static str,
    arch: &'static str,
    version: &'static str,
    limits: OsLimits,
    native_dialogs: bool,
    initial_config: Option<BenchmarkConfigInput>,
    mode: GuiMode,
}

#[derive(Debug, Serialize)]
struct ValidateRunResponse {
    valid: bool,
    field_errors: std::collections::HashMap<String, String>,
    warnings: Vec<String>,
    cli_command: String,
}

#[derive(Debug, Deserialize)]
struct StartRunRequest {
    config: BenchmarkConfigInput,
    #[serde(default)]
    preset_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SaveJsonDialogRequest {
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenBodyFileResponse {
    cancelled: bool,
    path: Option<String>,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct SaveJsonDialogResponse {
    cancelled: bool,
    path: Option<String>,
}

/// Builds the Axum [`Router`] for the interactive desktop / web configuration & telemetry UI.
pub fn build_interactive_router(state: InteractiveState) -> Router {
    let protected = Router::new()
        .route("/api/runs/validate", post(post_validate_run))
        .route("/api/runs", get(get_runs).post(post_start_run))
        .route("/api/runs/{id}", get(get_run_detail))
        .route("/api/stop", post(post_stop_run))
        .route("/api/export/json", get(export_interactive_json))
        .route("/api/presets", get(get_presets).post(post_create_preset))
        .route(
            "/api/presets/{id}",
            put(put_update_preset).delete(delete_preset),
        )
        .route("/api/dialog/open-body-file", post(post_open_body_file))
        .route("/api/dialog/save-json", post(post_save_json_dialog))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session_auth_middleware,
        ));

    Router::new()
        .route("/", get(serve_interactive_index))
        .route("/assets/styles.css", get(serve_styles))
        .route("/assets/app.js", get(serve_app_js))
        .route("/api/about", get(get_about))
        .route("/api/changelog", get(get_changelog))
        .route("/api/system", get(get_system))
        .route("/api/meta", get(get_meta))
        .route("/api/history", get(get_history))
        .route("/api/stream", get(stream_interactive_telemetry))
        .merge(protected)
        .with_state(state)
}

async fn session_auth_middleware(
    State(state): State<InteractiveState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if let Err(resp) = require_session(req.headers(), &state.session_token) {
        return resp;
    }
    next.run(req).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    if let Some(val) = headers.get(SESSION_HEADER) {
        if let Ok(s) = val.to_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    for cookie_hdr in headers.get_all(header::COOKIE) {
        if let Ok(raw) = cookie_hdr.to_str() {
            for part in raw.split(';') {
                let trimmed = part.trim();
                if let Some(val) = trimmed.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")) {
                    let token = val.trim();
                    if !token.is_empty() {
                        return Some(token.to_string());
                    }
                }
            }
        }
    }

    None
}

fn require_session(headers: &HeaderMap, expected: &str) -> Result<(), Response> {
    match extract_session_token(headers) {
        Some(provided) if constant_time_eq(provided.as_bytes(), expected.as_bytes()) => Ok(()),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "missing or invalid session token"
            })),
        )
            .into_response()),
    }
}

async fn serve_interactive_index(
    State(state): State<InteractiveState>,
    Query(query): Query<BootstrapQuery>,
) -> Response {
    if let Some(ref provided) = query.token {
        if constant_time_eq(provided.trim().as_bytes(), state.session_token.as_bytes()) {
            let cookie = format!(
                "{SESSION_COOKIE_NAME}={}; HttpOnly; SameSite=Strict; Path=/",
                state.session_token
            );
            let mut resp = (StatusCode::SEE_OTHER, "").into_response();
            if let Ok(cookie_val) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().insert(header::SET_COOKIE, cookie_val);
            }
            resp.headers_mut()
                .insert(header::LOCATION, HeaderValue::from_static("/"));
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            return resp;
        }
    }

    serve_index().await.into_response()
}

async fn get_about() -> impl IntoResponse {
    Json(about_payload())
}

async fn get_changelog() -> impl IntoResponse {
    Json(changelog_payload())
}

async fn get_system(State(state): State<InteractiveState>) -> Json<SystemInfoResponse> {
    Json(SystemInfoResponse {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        version: APP_VERSION,
        limits: OsLimits::detect(),
        native_dialogs: state.dialog_dispatcher.is_some(),
        initial_config: state.initial_config.clone(),
        mode: GuiMode::Interactive,
    })
}

async fn get_meta(State(state): State<InteractiveState>) -> Json<MetaResponse> {
    Json(state.manager.current_meta())
}

async fn get_history(
    State(state): State<InteractiveState>,
    Query(query): Query<RunIdQuery>,
) -> Json<Vec<TelemetrySnapshot>> {
    Json(state.manager.current_history(query.run_id.as_deref()))
}

async fn post_validate_run(
    Json(input): Json<BenchmarkConfigInput>,
) -> Response {
    let ValidationReport {
        valid,
        field_errors,
        warnings,
    } = validate_input(&input, Some(OsLimits::detect()));
    let cli_command = format_cli_command(&input, true);

    Json(ValidateRunResponse {
        valid,
        field_errors,
        warnings,
        cli_command,
    })
    .into_response()
}

async fn post_start_run(
    State(state): State<InteractiveState>,
    Json(req): Json<StartRunRequest>,
) -> Response {
    let config_for_recent = req.config.clone();
    let preset_for_recent = req.preset_name.clone();

    match state.manager.start_run(req.config, req.preset_name) {
        Ok((run_id, meta)) => {
            if let Err(err) = state
                .presets
                .record_recent(config_for_recent, preset_for_recent)
            {
                eprintln!("Warning: failed to record recent configuration: {err}");
            }
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "run_id": run_id,
                    "meta": meta,
                })),
            )
                .into_response()
        }
        Err(RunManagerError::RunAlreadyActive) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "a benchmark run is already in progress"
            })),
        )
            .into_response(),
        Err(RunManagerError::InvalidConfig(validation)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid benchmark configuration",
                "validation": validation,
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": err.to_string()
            })),
        )
            .into_response(),
    }
}

async fn post_stop_run(State(state): State<InteractiveState>) -> Response {
    match state.manager.stop_run() {
        Ok(run_id) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "stopping",
                "cancelled": true,
                "run_id": run_id,
            })),
        )
            .into_response(),
        Err(RunManagerError::NoActiveRun) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "no active benchmark run to stop"
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": err.to_string()
            })),
        )
            .into_response(),
    }
}

async fn get_runs(State(state): State<InteractiveState>) -> Response {
    Json(state.manager.list_runs()).into_response()
}

async fn get_run_detail(
    State(state): State<InteractiveState>,
    Path(id): Path<String>,
) -> Response {
    match state.manager.get_run(&id) {
        Some(record) => Json(record).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("run '{id}' not found") })),
        )
            .into_response(),
    }
}

async fn export_interactive_json(
    State(state): State<InteractiveState>,
    Query(query): Query<RunIdQuery>,
) -> Response {
    let Some(report) = state.manager.export_report(query.run_id.as_deref()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no benchmark report available to export" })),
        )
            .into_response();
    };

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

async fn get_presets(State(state): State<InteractiveState>) -> Response {
    Json(state.presets.list()).into_response()
}

async fn post_create_preset(
    State(state): State<InteractiveState>,
    Json(req): Json<PresetUpsertRequest>,
) -> Response {
    match state.presets.create(req) {
        Ok(preset) => (StatusCode::CREATED, Json(preset)).into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn put_update_preset(
    State(state): State<InteractiveState>,
    Path(id): Path<String>,
    Json(req): Json<PresetUpsertRequest>,
) -> Response {
    match state.presets.update(&id, req) {
        Ok(Some(updated)) => Json(updated).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("preset '{id}' not found") })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn delete_preset(
    State(state): State<InteractiveState>,
    Path(id): Path<String>,
) -> Response {
    match state.presets.delete(&id) {
        Ok(true) => Json(serde_json::json!({ "deleted": true })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("preset '{id}' not found") })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn post_open_body_file(State(state): State<InteractiveState>) -> Response {
    let Some(ref dispatcher) = state.dialog_dispatcher else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "error": "native file dialogs are only available in the macOS desktop host"
            })),
        )
            .into_response();
    };

    let (tx, rx) = oneshot::channel();
    if !dispatcher(NativeDialogRequest::OpenBodyFile { resp: tx }) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "desktop window event loop is unavailable" })),
        )
            .into_response();
    }

    match rx.await {
        Ok(Some(path)) => {
            let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Json(OpenBodyFileResponse {
                cancelled: false,
                path: Some(path.to_string_lossy().into_owned()),
                size_bytes,
            })
            .into_response()
        }
        Ok(None) => Json(OpenBodyFileResponse {
            cancelled: true,
            path: None,
            size_bytes: 0,
        })
        .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "native file dialog failed to respond" })),
        )
            .into_response(),
    }
}

async fn post_save_json_dialog(
    State(state): State<InteractiveState>,
    payload: Option<Json<SaveJsonDialogRequest>>,
) -> Response {
    let Some(ref dispatcher) = state.dialog_dispatcher else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "error": "native file dialogs are only available in the macOS desktop host"
            })),
        )
            .into_response();
    };

    let run_id = payload.and_then(|Json(req)| req.run_id);
    let Some(report) = state.manager.export_report(run_id.as_deref()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no benchmark report available to export" })),
        )
            .into_response();
    };

    let json_text = match serde_json::to_string_pretty(&report) {
        Ok(text) => text,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to serialize report: {err}") })),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel();
    if !dispatcher(NativeDialogRequest::SaveJsonFile {
        default_name: "rusty-loader-report.json".to_string(),
        resp: tx,
    }) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "desktop window event loop is unavailable" })),
        )
            .into_response();
    }

    match rx.await {
        Ok(Some(path)) => {
            if let Err(err) = fs::write(&path, &json_text) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("failed to write '{}': {err}", path.display())
                    })),
                )
                    .into_response();
            }
            Json(SaveJsonDialogResponse {
                cancelled: false,
                path: Some(path.to_string_lossy().into_owned()),
            })
            .into_response()
        }
        Ok(None) => Json(SaveJsonDialogResponse {
            cancelled: true,
            path: None,
        })
        .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "native save dialog failed to respond" })),
        )
            .into_response(),
    }
}

struct InteractiveSseState {
    manager: Arc<RunManager>,
    lifecycle_rx: tokio::sync::broadcast::Receiver<RunLifecycleEvent>,
    snap_rx: Option<tokio::sync::broadcast::Receiver<Arc<TelemetrySnapshot>>>,
    pending: VecDeque<Event>,
    done: bool,
}

fn snapshot_to_event(snapshot: &TelemetrySnapshot) -> Option<Event> {
    Event::default()
        .event("snapshot")
        .json_data(snapshot)
        .ok()
}

fn flush_pending_snapshots(
    snap_rx: &mut Option<tokio::sync::broadcast::Receiver<Arc<TelemetrySnapshot>>>,
    pending: &mut VecDeque<Event>,
) {
    if let Some(ref mut rx) = snap_rx {
        while let Ok(snap) = rx.try_recv() {
            if let Some(ev) = snapshot_to_event(&snap) {
                pending.push_back(ev);
            }
        }
    }
    *snap_rx = None;
}

async fn recv_snapshot_opt(
    snap_rx: &mut Option<tokio::sync::broadcast::Receiver<Arc<TelemetrySnapshot>>>,
) -> Result<Arc<TelemetrySnapshot>, RecvError> {
    match snap_rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn stream_interactive_telemetry(
    State(state): State<InteractiveState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let lifecycle_rx = state.manager.subscribe_lifecycle();
    let initial_meta = state.manager.current_meta();
    let initial_history = state.manager.current_history(None);
    let snap_rx = state
        .manager
        .active_telemetry()
        .map(|(_, hub, _)| hub.subscribe());

    let mut pending = VecDeque::new();
    if let Ok(ev) = Event::default().event("state").json_data(serde_json::json!({
        "meta": initial_meta,
        "history": initial_history,
    })) {
        pending.push_back(ev);
    }

    let initial = InteractiveSseState {
        manager: Arc::clone(&state.manager),
        lifecycle_rx,
        snap_rx,
        pending,
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

            tokio::select! {
                biased;
                res = recv_snapshot_opt(&mut s.snap_rx) => {
                    match res {
                        Ok(snap) => {
                            if let Some(ev) = snapshot_to_event(&snap) {
                                return Some((Ok(ev), s));
                            }
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => {
                            s.snap_rx = None;
                        }
                    }
                }
                ev_res = s.lifecycle_rx.recv() => {
                    match ev_res {
                        Ok(RunLifecycleEvent::RunStarted { run_id, meta }) => {
                            if let Some((active_id, hub, _)) = s.manager.active_telemetry() {
                                if active_id == run_id {
                                    s.snap_rx = Some(hub.subscribe());
                                }
                            }
                            if let Ok(ev) = Event::default().event("run_started").json_data(serde_json::json!({
                                "run_id": run_id,
                                "meta": meta,
                            })) {
                                s.pending.push_back(ev);
                            }
                        }
                        Ok(RunLifecycleEvent::RunCompleted { run_id, stats, meta }) => {
                            flush_pending_snapshots(&mut s.snap_rx, &mut s.pending);
                            if let Ok(ev) = Event::default().event("complete").json_data(serde_json::json!({
                                "run_id": run_id,
                                "stats": stats,
                                "meta": meta,
                            })) {
                                s.pending.push_back(ev);
                            }
                        }
                        Ok(RunLifecycleEvent::RunFailed { run_id, error, meta }) => {
                            flush_pending_snapshots(&mut s.snap_rx, &mut s.pending);
                            if let Ok(ev) = Event::default().event("run_failed").json_data(serde_json::json!({
                                "run_id": run_id,
                                "error": error,
                                "meta": meta,
                            })) {
                                s.pending.push_back(ev);
                            }
                        }
                        Err(RecvError::Lagged(_)) => {
                            let meta = s.manager.current_meta();
                            let history = s.manager.current_history(None);
                            if let Some((_, hub, _)) = s.manager.active_telemetry() {
                                s.snap_rx = Some(hub.subscribe());
                            }
                            if let Ok(ev) = Event::default().event("state").json_data(serde_json::json!({
                                "meta": meta,
                                "history": history,
                            })) {
                                s.pending.push_back(ev);
                            }
                        }
                        Err(RecvError::Closed) => {
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
