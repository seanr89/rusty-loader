#![cfg(feature = "gui")]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use rusty_loader::cli::HttpMethod;
use rusty_loader::config::{BenchmarkConfigInput, BodyMode, HeaderEntry, ValidationReport};
use rusty_loader::gui::{
    start_interactive_gui_server, CompletedRunRecord, CompletedRunSummary, InteractiveServerHandle,
    MetaResponse, RunManager,
};
use rusty_loader::metrics::{BenchmarkStats, TelemetryPhase};
use rusty_loader::presets::{PresetStore, PresetsPayload, SavedPreset};
use rusty_loader::reporter::JsonReport;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn local_client(follow_redirects: bool) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5));
    if !follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    builder.build().expect("local reqwest client")
}

async fn start_test_interactive_server() -> (
    InteractiveServerHandle,
    Arc<RunManager>,
    Arc<PresetStore>,
    TempDir,
) {
    let dir = TempDir::new().expect("temp dir");
    let presets = Arc::new(
        PresetStore::load_from_dir(dir.path().to_path_buf()).expect("load temp presets store"),
    );
    let manager = Arc::new(RunManager::new());
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    let handle = start_interactive_gui_server(
        addr,
        Arc::clone(&manager),
        Arc::clone(&presets),
        None,
        None,
    )
    .await
    .expect("interactive server should start");

    (handle, manager, presets, dir)
}

async fn read_sse_until_two_completes(
    mut response: reqwest::Response,
    timeout_dur: Duration,
) -> Vec<(String, String)> {
    let mut buffer = String::new();
    let mut events = Vec::new();
    let mut complete_count = 0usize;

    let read_fut = async {
        while let Ok(Some(chunk)) = response.chunk().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(sep_idx) = buffer.find("\n\n") {
                let raw_frame = buffer[..sep_idx].to_string();
                buffer.drain(..sep_idx + 2);

                let mut event_name = String::from("message");
                let mut data_lines = Vec::new();
                for line in raw_frame.lines() {
                    let line = line.trim_end_matches('\r');
                    if line.starts_with(':') || line.is_empty() {
                        continue;
                    }
                    if let Some(val) = line.strip_prefix("event:") {
                        event_name = val.trim().to_string();
                    } else if let Some(val) = line.strip_prefix("data:") {
                        data_lines.push(val.trim_start().to_string());
                    }
                }

                if !data_lines.is_empty() {
                    let payload = data_lines.join("\n");
                    if event_name == "complete" {
                        complete_count += 1;
                    }
                    events.push((event_name, payload));
                    if complete_count >= 2 {
                        return events;
                    }
                }
            }
        }
        events
    };

    tokio::time::timeout(timeout_dur, read_fut)
        .await
        .expect("timed out waiting for multi-run SSE events")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_interactive_session_auth_and_bootstrap() {
    let (handle, manager, _presets, _dir) = start_test_interactive_server().await;
    let base_url = handle.url();
    let unauth_client = local_client(true);

    // Public read metadata returns idle interactive state
    let meta: MetaResponse = unauth_client
        .get(format!("{base_url}/api/meta"))
        .send()
        .await
        .expect("GET /api/meta")
        .json()
        .await
        .expect("valid meta JSON");
    assert_eq!(meta.phase, TelemetryPhase::Idle);

    // Mutating and sensitive endpoints reject missing token with 401
    for (m, p) in [
        ("POST", "/api/runs/validate"),
        ("POST", "/api/runs"),
        ("POST", "/api/stop"),
        ("GET", "/api/presets"),
        ("POST", "/api/presets"),
        ("GET", "/api/runs"),
        ("GET", "/api/export/json"),
        ("POST", "/api/dialog/open-body-file"),
    ] {
        let req = match m {
            "POST" => unauth_client
                .post(format!("{base_url}{p}"))
                .json(&serde_json::json!({})),
            _ => unauth_client.get(format!("{base_url}{p}")),
        };
        let resp = req.send().await.expect("unauth request");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "expected 401 for unauthenticated {m} {p}"
        );
    }

    // Header authentication works; desktop-only native dialog returns 501 when dialog bridge is None
    let dialog_resp = unauth_client
        .post(format!("{base_url}/api/dialog/open-body-file"))
        .header("X-RL-Session", handle.session_token())
        .send()
        .await
        .expect("dialog call with header token");
    assert_eq!(dialog_resp.status(), StatusCode::NOT_IMPLEMENTED);

    // Cookie bootstrap via tokenized URL sets HttpOnly SameSite=Strict cookie and redirects to /
    let no_redir_client = local_client(false);
    let boot_resp = no_redir_client
        .get(handle.tokenized_url())
        .send()
        .await
        .expect("bootstrap GET /?token=...");
    assert_eq!(boot_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        boot_resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/")
    );
    let set_cookie = boot_resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .expect("Set-Cookie header should be present");
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));
    let cookie_pair = set_cookie
        .split(';')
        .next()
        .expect("cookie key=value")
        .trim()
        .to_string();

    let presets_resp = unauth_client
        .get(format!("{base_url}/api/presets"))
        .header(reqwest::header::COOKIE, cookie_pair)
        .send()
        .await
        .expect("GET /api/presets with session cookie");
    assert_eq!(presets_resp.status(), StatusCode::OK);

    manager.shutdown().await;
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_interactive_validation_and_multi_run_lifecycle_with_sse() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("ok")
                .set_delay(Duration::from_millis(5)),
        )
        .mount(&mock_server)
        .await;

    let (handle, manager, _presets, _dir) = start_test_interactive_server().await;
    let base_url = handle.url();
    let token = handle.session_token().to_string();
    let client = local_client(true);

    // 1. Validate raw JSON payload (verifies uppercase method serialization/deserialization) and invalid config
    let valid_raw = serde_json::json!({
        "url": format!("{}/ok", mock_server.uri()),
        "method": "POST",
        "concurrency": 10,
        "duration": "5s",
        "ramp_up": "0s",
        "timeout": "1000ms",
        "headers": [],
        "body_mode": "raw",
        "body": "{\"hello\":\"world\"}"
    });
    let valid_raw_resp = client
        .post(format!("{base_url}/api/runs/validate"))
        .header("X-RL-Session", &token)
        .json(&valid_raw)
        .send()
        .await
        .expect("validate raw JSON config");
    assert_eq!(valid_raw_resp.status(), StatusCode::OK);
    let valid_raw_json: serde_json::Value = valid_raw_resp.json().await.unwrap();
    assert_eq!(valid_raw_json["valid"], true);

    // Invalid config -> valid = false with field errors
    let bad_cfg = BenchmarkConfigInput {
        url: "ftp://invalid.example.com".to_string(),
        concurrency: 0,
        duration: "10s".to_string(),
        ramp_up: "20s".to_string(),
        timeout: "0ms".to_string(),
        ..Default::default()
    };
    let bad_val_resp = client
        .post(format!("{base_url}/api/runs/validate"))
        .header("X-RL-Session", &token)
        .json(&bad_cfg)
        .send()
        .await
        .expect("validate bad config");
    assert_eq!(bad_val_resp.status(), StatusCode::OK);
    let bad_val_json: serde_json::Value = bad_val_resp.json().await.unwrap();
    assert_eq!(bad_val_json["valid"], false);
    assert!(bad_val_json["field_errors"]["url"].is_string());
    assert!(bad_val_json["field_errors"]["concurrency"].is_string());
    assert!(bad_val_json["field_errors"]["ramp_up"].is_string());

    // Invalid POST /api/runs returns 400
    let start_bad = client
        .post(format!("{base_url}/api/runs"))
        .header("X-RL-Session", &token)
        .json(&serde_json::json!({ "config": bad_cfg }))
        .send()
        .await
        .expect("start bad config");
    assert_eq!(start_bad.status(), StatusCode::BAD_REQUEST);
    let validation: ValidationReport =
        serde_json::from_value(start_bad.json::<serde_json::Value>().await.unwrap()["validation"].clone())
            .unwrap();
    assert!(!validation.valid);

    // 2. Connect persistent multi-run SSE stream
    let sse_resp = client
        .get(format!("{base_url}/api/stream"))
        .send()
        .await
        .expect("connect SSE");
    assert_eq!(sse_resp.status(), StatusCode::OK);
    let sse_task = tokio::spawn(read_sse_until_two_completes(
        sse_resp,
        Duration::from_secs(10),
    ));

    // 3. Start Run 1 (500ms) with sensitive Authorization header
    let run1_cfg = BenchmarkConfigInput {
        url: format!("{}/ok", mock_server.uri()),
        method: HttpMethod::Get,
        concurrency: 6,
        duration: "500ms".to_string(),
        ramp_up: "0s".to_string(),
        timeout: "2s".to_string(),
        headers: vec![HeaderEntry {
            name: "Authorization".to_string(),
            value: "Bearer top-secret-run1".to_string(),
            enabled: true,
        }],
        body_mode: BodyMode::None,
        body: None,
        body_file: None,
        output_json: None,
    };

    let start1 = client
        .post(format!("{base_url}/api/runs"))
        .header("X-RL-Session", &token)
        .json(&serde_json::json!({ "config": run1_cfg, "preset_name": "Smoke Run 1" }))
        .send()
        .await
        .expect("start run 1");
    assert_eq!(start1.status(), StatusCode::ACCEPTED);
    let run1_id = start1.json::<serde_json::Value>().await.unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Starting an overlapping run while Run 1 is active must return 409 Conflict
    let overlap = client
        .post(format!("{base_url}/api/runs"))
        .header("X-RL-Session", &token)
        .json(&serde_json::json!({ "config": run1_cfg }))
        .send()
        .await
        .expect("overlapping start");
    assert_eq!(overlap.status(), StatusCode::CONFLICT);

    // Wait for Run 1 to finish and appear in /api/runs
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let runs: Vec<CompletedRunSummary> = client
            .get(format!("{base_url}/api/runs"))
            .header("X-RL-Session", &token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if runs.len() == 1 {
            assert_eq!(runs[0].run_id, run1_id);
            assert_eq!(runs[0].preset_name.as_deref(), Some("Smoke Run 1"));
            assert!(runs[0].total_requests > 0);
            break;
        }
        assert!(Instant::now() < deadline, "timed out waiting for Run 1");
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    // Verify Run 1 export redacts Authorization header
    let export_report: JsonReport = client
        .get(format!("{base_url}/api/export/json?run_id={run1_id}"))
        .header("X-RL-Session", &token)
        .send()
        .await
        .expect("export run 1")
        .json()
        .await
        .expect("valid JsonReport");
    assert_eq!(
        export_report.headers.get("authorization").map(String::as_str),
        Some("[REDACTED]")
    );

    // 4. Start Run 2 sequentially in the same app session
    let mut run2_cfg = run1_cfg.clone();
    run2_cfg.duration = "400ms".to_string();
    let start2 = client
        .post(format!("{base_url}/api/runs"))
        .header("X-RL-Session", &token)
        .json(&serde_json::json!({ "config": run2_cfg, "preset_name": "Smoke Run 2" }))
        .send()
        .await
        .expect("start run 2");
    assert_eq!(start2.status(), StatusCode::ACCEPTED);

    let sse_events = sse_task.await.expect("SSE task should complete cleanly");
    let started_count = sse_events
        .iter()
        .filter(|(name, _)| name == "run_started")
        .count();
    let complete_events: Vec<&String> = sse_events
        .iter()
        .filter(|(name, _)| name == "complete")
        .map(|(_, data)| data)
        .collect();
    assert!(
        sse_events.iter().any(|(name, _)| name == "state"),
        "expected initial state SSE event"
    );
    assert_eq!(started_count, 2, "expected 2 run_started SSE events");
    assert_eq!(complete_events.len(), 2, "expected 2 complete SSE events");

    for payload in complete_events {
        let val: serde_json::Value = serde_json::from_str(payload).unwrap();
        let stats: BenchmarkStats = serde_json::from_value(val["stats"].clone()).unwrap();
        assert!(stats.total_requests > 0);
    }

    let runs: Vec<CompletedRunSummary> = client
        .get(format!("{base_url}/api/runs"))
        .header("X-RL-Session", &token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(runs.len(), 2);

    manager.shutdown().await;
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_interactive_stop_run_and_presets_http_crud() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/work"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("ok")
                .set_delay(Duration::from_millis(5)),
        )
        .mount(&mock_server)
        .await;

    let (handle, manager, _presets, _dir) = start_test_interactive_server().await;
    let base_url = handle.url();
    let token = handle.session_token().to_string();
    let client = local_client(true);

    // 1. Preset CRUD over HTTP
    let cfg = BenchmarkConfigInput {
        url: format!("{}/work", mock_server.uri()),
        concurrency: 6,
        duration: "10s".to_string(),
        ..Default::default()
    };

    let created: SavedPreset = client
        .post(format!("{base_url}/api/presets"))
        .header("X-RL-Session", &token)
        .json(&serde_json::json!({
            "name": "HTTP Preset",
            "description": "Created over REST",
            "config": cfg
        }))
        .send()
        .await
        .expect("POST /api/presets")
        .json()
        .await
        .unwrap();
    assert_eq!(created.name, "HTTP Preset");

    let list_before: PresetsPayload = client
        .get(format!("{base_url}/api/presets"))
        .header("X-RL-Session", &token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list_before.presets.len(), 1);

    // 2. Start long run and stop mid-run via POST /api/stop
    let started = Instant::now();
    let start_resp = client
        .post(format!("{base_url}/api/runs"))
        .header("X-RL-Session", &token)
        .json(&serde_json::json!({ "config": created.config, "preset_name": created.name }))
        .send()
        .await
        .expect("start long run");
    assert_eq!(start_resp.status(), StatusCode::ACCEPTED);
    let run_id = start_resp.json::<serde_json::Value>().await.unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string();

    tokio::time::sleep(Duration::from_millis(120)).await;

    let stop_resp = client
        .post(format!("{base_url}/api/stop"))
        .header("X-RL-Session", &token)
        .send()
        .await
        .expect("POST /api/stop");
    assert_eq!(stop_resp.status(), StatusCode::OK);

    // Wait for cancelled run to archive
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let detail_resp = client
            .get(format!("{base_url}/api/runs/{run_id}"))
            .header("X-RL-Session", &token)
            .send()
            .await
            .unwrap();
        if detail_resp.status() == StatusCode::OK {
            let record: CompletedRunRecord = detail_resp.json().await.unwrap();
            assert!(record.stats.expect("partial stats").total_requests > 0);
            assert!(started.elapsed() < Duration::from_secs(3));
            break;
        }
        assert!(Instant::now() < deadline, "timed out waiting for stopped run");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // Delete preset over HTTP
    let del_resp = client
        .delete(format!("{base_url}/api/presets/{}", created.id))
        .header("X-RL-Session", &token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), StatusCode::OK);

    manager.shutdown().await;
    handle.shutdown().await;
}
