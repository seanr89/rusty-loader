use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rusty_loader::cli::HttpMethod;
use rusty_loader::config::{
    validate_input_with_review, BenchmarkConfig, BenchmarkConfigInput, BodyMode, HeaderEntry,
};
use rusty_loader::engine::TestController;
use rusty_loader::gui::{start_interactive_gui_server, RunManager};
use rusty_loader::metrics::TelemetryHub;
use rusty_loader::presets::PresetStore;
use rusty_loader::reporter::{
    generate_pdf_report, render_summary_tables, serialize_csv_report, serialize_json_report,
    ExportReport,
};
use rusty_loader::scenario::{
    execute_dry_run, ApiStepInput, DryRunTraceReport, ExtractorSource, JsonCheckInput, JsonCheckOp,
    RequestMode, ScenarioConfigInput, ScenarioVariableInput, StatusExpectation,
    StepAssertionsInput, StepExtractorInput, StepFailurePolicy,
};

fn build_checkout_scenario_input(
    base_url: &str,
    step1_on_failure: StepFailurePolicy,
) -> BenchmarkConfigInput {
    BenchmarkConfigInput {
        mode: RequestMode::Sequence,
        concurrency: 4,
        duration: "350ms".to_string(),
        ramp_up: "0s".to_string(),
        timeout: "2000ms".to_string(),
        scenario: Some(ScenarioConfigInput {
            schema_version: 1,
            name: Some("Checkout Sequence".to_string()),
            base_url: Some(base_url.to_string()),
            default_headers: vec![HeaderEntry {
                name: "Accept".to_string(),
                value: "application/json".to_string(),
                enabled: true,
            }],
            initial_variables: vec![ScenarioVariableInput {
                name: "login_email".to_string(),
                value: "user@example.com".to_string(),
                sensitive: false,
            }],
            steps: vec![
                ApiStepInput {
                    id: "step_login".to_string(),
                    name: "Login".to_string(),
                    enabled: true,
                    method: HttpMethod::Post,
                    url: "/auth/login".to_string(),
                    headers: Vec::new(),
                    body_mode: BodyMode::Raw,
                    body: Some(r#"{"email":"{{login_email}}","vu":{{__vu_id}}}"#.to_string()),
                    body_file: None,
                    timeout: None,
                    think_time: None,
                    extractors: vec![
                        StepExtractorInput {
                            name: "auth_token".to_string(),
                            source: ExtractorSource::JsonPath {
                                path: "$.data.access_token".to_string(),
                            },
                            required: true,
                            sensitive: true,
                        },
                        StepExtractorInput {
                            name: "user_id".to_string(),
                            source: ExtractorSource::JsonPath {
                                path: "$.data.user.id".to_string(),
                            },
                            required: true,
                            sensitive: false,
                        },
                    ],
                    assertions: StepAssertionsInput {
                        status: StatusExpectation::Exact { codes: vec![200] },
                        json_checks: Vec::new(),
                    },
                    on_failure: step1_on_failure,
                },
                ApiStepInput {
                    id: "step_cart".to_string(),
                    name: "Create Cart".to_string(),
                    enabled: true,
                    method: HttpMethod::Post,
                    url: "/users/{{user_id}}/carts".to_string(),
                    headers: vec![HeaderEntry {
                        name: "Authorization".to_string(),
                        value: "Bearer {{auth_token}}".to_string(),
                        enabled: true,
                    }],
                    body_mode: BodyMode::Raw,
                    body: Some(r#"{"sku":"sku-100"}"#.to_string()),
                    body_file: None,
                    timeout: None,
                    think_time: None,
                    extractors: vec![StepExtractorInput {
                        name: "cart_id".to_string(),
                        source: ExtractorSource::JsonPath {
                            path: "$.cart.id".to_string(),
                        },
                        required: true,
                        sensitive: false,
                    }],
                    assertions: StepAssertionsInput {
                        status: StatusExpectation::Any2xx,
                        json_checks: Vec::new(),
                    },
                    on_failure: StepFailurePolicy::AbortIteration,
                },
                ApiStepInput {
                    id: "step_checkout".to_string(),
                    name: "Checkout".to_string(),
                    enabled: true,
                    method: HttpMethod::Post,
                    url: "/carts/{{cart_id}}/checkout".to_string(),
                    headers: vec![HeaderEntry {
                        name: "Authorization".to_string(),
                        value: "Bearer {{auth_token}}".to_string(),
                        enabled: true,
                    }],
                    body_mode: BodyMode::None,
                    body: None,
                    body_file: None,
                    timeout: None,
                    think_time: None,
                    extractors: Vec::new(),
                    assertions: StepAssertionsInput {
                        status: StatusExpectation::Exact { codes: vec![201] },
                        json_checks: vec![JsonCheckInput {
                            path: "$.order.status".to_string(),
                            op: JsonCheckOp::Equals,
                            expected: Some(json!("confirmed")),
                        }],
                    },
                    on_failure: StepFailurePolicy::AbortIteration,
                },
            ],
        }),
        ..Default::default()
    }
}

async fn mount_checkout_mocks(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .and(header("accept", "application/json"))
        .and(body_string_contains("user@example.com"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "access_token": "secret-jwt-token-value-999",
                "user": { "id": "usr_42" }
            }
        })))
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(path("/users/usr_42/carts"))
        .and(header("authorization", "Bearer secret-jwt-token-value-999"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "cart": { "id": "cart_777" }
        })))
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(path("/carts/cart_777/checkout"))
        .and(header("authorization", "Bearer secret-jwt-token-value-999"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "order": { "status": "confirmed" }
        })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_three_step_sequence_execution_telemetry_and_exports() {
    let server = MockServer::start().await;
    mount_checkout_mocks(&server).await;

    let input = build_checkout_scenario_input(&server.uri(), StepFailurePolicy::AbortIteration);
    let config = BenchmarkConfig::from_input(input).expect("compile sequence config");
    let hub = Arc::new(TelemetryHub::with_options(
        config.duration,
        config.ramp_up,
        Duration::from_millis(50),
        100,
        32,
    ));

    let controller = TestController::from_config(config.clone())
        .expect("controller builds")
        .with_signal_handler(false)
        .with_telemetry_hub(Arc::clone(&hub));

    let stats = controller.run().await.expect("sequence run succeeds");
    assert!(stats.total_requests >= 3);
    assert_eq!(stats.successful_requests, stats.total_requests);

    let sc = stats
        .scenario_stats
        .as_ref()
        .expect("scenario_stats present");
    assert_eq!(sc.steps.len(), 3);
    assert!(sc.journeys.completed_iterations >= 1);
    assert_eq!(sc.journeys.aborted_iterations, 0);
    assert!((sc.journeys.completion_rate_pct - 100.0).abs() < 1e-6);
    assert_eq!(sc.steps[0].step_name, "Login");
    assert_eq!(sc.steps[1].step_name, "Create Cart");
    assert_eq!(sc.steps[2].step_name, "Checkout");
    assert!(sc.steps[0].total_requests >= sc.steps[1].total_requests);
    assert!(sc.steps[1].total_requests >= sc.steps[2].total_requests);
    assert_eq!(sc.journeys.completed_iterations, sc.steps[2].total_requests);
    assert!(sc.steps[0].total_requests - sc.steps[2].total_requests <= 4);

    // Terminal summary tables render per-step and journey sections
    let rendered = render_summary_tables(&config, &stats);
    assert!(rendered.contains("Sequence Journey Summary"));
    assert!(rendered.contains("Per-Step Sequence Breakdown"));
    assert!(rendered.contains("Checkout"));

    // JSON, CSV, and PDF exports include scenario metadata/rows and preserve redaction
    let snapshots: Vec<_> = hub.history().into_iter().map(|s| (*s).clone()).collect();
    let export = ExportReport::from_config_redacted(&config, &stats, snapshots, None);
    let json_text = serialize_json_report(&config, &stats).expect("json report");
    assert!(json_text.contains("\"scenario_stats\""));
    assert!(json_text.contains("\"Checkout\""));
    assert!(!json_text.contains("secret-jwt-token-value-999"));

    let csv_text = serialize_csv_report(&export).expect("csv export");
    assert!(csv_text.contains("step_summary"));
    assert!(csv_text.contains("journey_summary"));
    for rec in csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_text.as_bytes())
        .records()
    {
        assert_eq!(rec.expect("valid csv record").len(), 26);
    }

    let pdf_bytes = generate_pdf_report(&export).expect("pdf export");
    assert!(pdf_bytes.starts_with(b"%PDF-1.7"));
    let pdf_lossy = String::from_utf8_lossy(&pdf_bytes);
    assert!(pdf_lossy.contains("Ordered API Sequence: Journey & Per-Step Breakdown"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_step_failure_policy_abort_vs_continue() {
    // 1. AbortIteration skips Step 2 & Step 3 when Step 1 returns 401
    let server_abort = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server_abort)
        .await;

    let mut cfg_abort_input =
        build_checkout_scenario_input(&server_abort.uri(), StepFailurePolicy::AbortIteration);
    cfg_abort_input.concurrency = 2;
    cfg_abort_input.duration = "200ms".to_string();
    let cfg_abort = BenchmarkConfig::from_input(cfg_abort_input).unwrap();
    let stats_abort = TestController::from_config(cfg_abort)
        .unwrap()
        .with_signal_handler(false)
        .run()
        .await
        .unwrap();

    let sc_abort = stats_abort.scenario_stats.unwrap();
    assert!(sc_abort.steps[0].total_requests >= 1);
    assert_eq!(sc_abort.steps[1].total_requests, 0);
    assert_eq!(sc_abort.steps[2].total_requests, 0);
    assert!(sc_abort.journeys.aborted_iterations >= 1);
    assert_eq!(sc_abort.journeys.completed_iterations, 0);
    assert!(sc_abort.journeys.aborted_by_step.get(&0).copied().unwrap_or(0) >= 1);

    // 2. Continue allows downstream steps to run even when Step 1 fails
    let server_cont = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(ResponseTemplate::new(500).set_body_string("err"))
        .mount(&server_cont)
        .await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server_cont)
        .await;

    let cont_input = BenchmarkConfigInput {
        mode: RequestMode::Sequence,
        concurrency: 1,
        duration: "200ms".to_string(),
        scenario: Some(ScenarioConfigInput {
            schema_version: 1,
            name: Some("Continue Policy".to_string()),
            base_url: Some(server_cont.uri()),
            steps: vec![
                ApiStepInput {
                    id: "s1".to_string(),
                    name: "Fail But Continue".to_string(),
                    enabled: true,
                    method: HttpMethod::Post,
                    url: "/auth/login".to_string(),
                    headers: Vec::new(),
                    body_mode: BodyMode::None,
                    body: None,
                    body_file: None,
                    timeout: None,
                    think_time: None,
                    extractors: Vec::new(),
                    assertions: StepAssertionsInput::default(),
                    on_failure: StepFailurePolicy::Continue,
                },
                ApiStepInput {
                    id: "s2".to_string(),
                    name: "Health".to_string(),
                    enabled: true,
                    method: HttpMethod::Get,
                    url: "/health".to_string(),
                    headers: Vec::new(),
                    body_mode: BodyMode::None,
                    body: None,
                    body_file: None,
                    timeout: None,
                    think_time: None,
                    extractors: Vec::new(),
                    assertions: StepAssertionsInput::default(),
                    on_failure: StepFailurePolicy::AbortIteration,
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg_cont = BenchmarkConfig::from_input(cont_input).unwrap();
    let stats_cont = TestController::from_config(cfg_cont)
        .unwrap()
        .with_signal_handler(false)
        .run()
        .await
        .unwrap();

    let sc_cont = stats_cont.scenario_stats.unwrap();
    assert!(sc_cont.steps[0].total_requests >= 1);
    assert!(sc_cont.steps[1].total_requests >= 1);
    assert!(sc_cont.journeys.completed_iterations >= 1);
}

#[tokio::test]
async fn test_dry_run_redacts_secrets_and_scrubs_preview() {
    let server = MockServer::start().await;
    mount_checkout_mocks(&server).await;

    let input = build_checkout_scenario_input(&server.uri(), StepFailurePolicy::AbortIteration);
    let config = BenchmarkConfig::from_input(input).expect("compile config");
    let trace = execute_dry_run(&config).await.expect("dry run executes");

    assert!(trace.completed);
    assert!(trace.aborted_at_step.is_none());
    assert_eq!(trace.steps.len(), 3);
    assert!(trace.steps.iter().all(|s| s.passed));

    // Sensitive extracted variable must be masked and raw token scrubbed from body preview
    let auth_var = trace.steps[0]
        .extracted
        .iter()
        .find(|v| v.name == "auth_token")
        .expect("auth_token extracted");
    assert!(auth_var.sensitive);
    assert!(auth_var.value.starts_with("[REDACTED (len="));
    assert!(!auth_var.value.contains("secret-jwt-token-value-999"));

    let preview = trace.steps[0]
        .response_body_preview
        .as_deref()
        .unwrap_or("");
    assert!(!preview.contains("secret-jwt-token-value-999"));
    assert!(preview.contains("[REDACTED]"));

    assert_eq!(
        trace.steps[1]
            .request_headers
            .get("authorization")
            .map(String::as_str),
        Some("[REDACTED]")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_interactive_api_validate_dry_run_and_sequence_execution() {
    let server = MockServer::start().await;
    mount_checkout_mocks(&server).await;

    let temp_dir = TempDir::new().unwrap();
    let presets = Arc::new(PresetStore::load_from_dir(temp_dir.path().to_path_buf()).unwrap());
    let manager = Arc::new(RunManager::new());
    let handle = start_interactive_gui_server(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        manager,
        presets,
        None,
        None,
    )
    .await
    .unwrap();

    let base_url = handle.url();
    let token = handle.session_token().to_string();
    let client = Client::builder().no_proxy().build().unwrap();
    let cfg = build_checkout_scenario_input(&server.uri(), StepFailurePolicy::AbortIteration);

    // 1. Validate returns review matrix
    let val_resp: serde_json::Value = client
        .post(format!("{base_url}/api/runs/validate"))
        .header("X-RL-Session", &token)
        .json(&cfg)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(val_resp["valid"], true);
    assert_eq!(val_resp["review"]["enabled_step_count"], 3);
    assert_eq!(val_resp["review"]["steps"][1]["consumes"][0], "user_id");

    // 2. Dry-run requires session auth
    let unauth = client
        .post(format!("{base_url}/api/scenarios/dry-run"))
        .json(&cfg)
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let dry_resp = client
        .post(format!("{base_url}/api/scenarios/dry-run"))
        .header("X-RL-Session", &token)
        .json(&cfg)
        .send()
        .await
        .unwrap();
    assert_eq!(dry_resp.status(), reqwest::StatusCode::OK);
    let trace: DryRunTraceReport = dry_resp.json().await.unwrap();
    assert!(trace.completed);
    assert_eq!(trace.steps.len(), 3);

    // 3. Invalid dependency caught cleanly in validation
    let mut bad_cfg = cfg.clone();
    bad_cfg.scenario.as_mut().unwrap().steps.swap(0, 1);
    let (bad_report, bad_review) = validate_input_with_review(&bad_cfg, None);
    assert!(!bad_report.valid);
    assert!(!bad_review.unwrap().valid);

    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cli_scenario_review_dry_run_and_flag_overrides() {
    let server = MockServer::start().await;
    mount_checkout_mocks(&server).await;

    let tmp_dir = TempDir::new().unwrap();
    let scenario_path = tmp_dir.path().join("checkout.json");
    let input = build_checkout_scenario_input(&server.uri(), StepFailurePolicy::AbortIteration);
    std::fs::write(&scenario_path, serde_json::to_string_pretty(&input).unwrap()).unwrap();

    // 1. --scenario --review
    let review_out = Command::new(env!("CARGO_BIN_EXE_rusty-loader"))
        .args(["--scenario", scenario_path.to_str().unwrap(), "--review"])
        .output()
        .expect("run --review");
    assert!(
        review_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&review_out.stderr)
    );
    let review_stdout = String::from_utf8_lossy(&review_out.stdout);
    assert!(review_stdout.contains("Sequence Review: Checkout Sequence"));
    assert!(review_stdout.contains("VALID"));

    // 2. --scenario --dry-run
    let dry_out = Command::new(env!("CARGO_BIN_EXE_rusty-loader"))
        .args(["--scenario", scenario_path.to_str().unwrap(), "--dry-run"])
        .output()
        .expect("run --dry-run");
    assert!(
        dry_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&dry_out.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_out.stdout);
    assert!(dry_stdout.contains("1-VU Dry-Run Trace (COMPLETED)"));
    assert!(!dry_stdout.contains("secret-jwt-token-value-999"));

    // 3. Explicit CLI flags (-c, -d, --output-json) override JSON settings
    let out_json = tmp_dir.path().join("override-report.json");
    let run_out = Command::new(env!("CARGO_BIN_EXE_rusty-loader"))
        .args([
            "--scenario",
            scenario_path.to_str().unwrap(),
            "-c",
            "2",
            "-d",
            "250ms",
            "--output-json",
            out_json.to_str().unwrap(),
        ])
        .output()
        .expect("run overridden scenario");
    assert!(
        run_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out_json).unwrap()).unwrap();
    assert_eq!(parsed["concurrency"], 2);
    assert_eq!(parsed["scenario"]["step_count"], 3);
    assert!(parsed["scenario_stats"]["journeys"]["completed_iterations"]
        .as_u64()
        .unwrap_or(0)
        >= 1);
}
