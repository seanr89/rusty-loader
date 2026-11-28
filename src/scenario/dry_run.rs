//! Single-VU dry-run executor producing redacted step-by-step execution traces.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Table};
use reqwest::redirect::Policy;
use reqwest::{Body, Client, Request};
use serde::{Deserialize, Serialize};

use crate::config::{redact_headers, sanitize_url, BenchmarkConfig};
use crate::metrics::snapshot::current_timestamp_ms;
use crate::scenario::config::{
    scrub_sensitive_text, CompiledScenario, CompiledStep, StepFailurePolicy,
};
use crate::scenario::extract::MAX_EXTRACTION_BODY_BYTES;
use crate::scenario::template::BuiltInVars;

/// Maximum UTF-8 preview bytes returned per step in a dry-run trace.
pub const DRY_RUN_PREVIEW_LIMIT: usize = 4096;

/// Overall wall-clock timeout applied to an interactive or CLI dry-run trace.
pub const DRY_RUN_OVERALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Extracted variable trace entry (sensitive values are masked).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunExtractedVar {
    pub name: String,
    pub source: String,
    pub value: String,
    pub sensitive: bool,
    pub found: bool,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Assertion outcome captured during dry-run execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunAssertionResult {
    pub description: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Step-by-step redacted diagnostic trace returned by `--dry-run` and `POST /api/scenarios/dry-run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunStepTrace {
    pub step_index: usize,
    pub order_number: usize,
    pub step_id: String,
    pub step_name: String,
    pub method: String,
    pub resolved_url: String,
    pub request_headers: HashMap<String, String>,
    pub status_code: Option<u16>,
    pub latency_ms: f64,
    pub bytes_received: usize,
    pub passed: bool,
    pub aborted_iteration: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub assertions: Vec<DryRunAssertionResult>,
    pub extracted: Vec<DryRunExtractedVar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_body_preview: Option<String>,
    pub response_body_truncated: bool,
}

/// Complete trace report for a single 1-VU iteration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunTraceReport {
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aborted_at_step: Option<usize>,
    pub total_duration_ms: f64,
    pub steps: Vec<DryRunStepTrace>,
}

/// Executes one 1-VU dry-run iteration of either a compiled sequence or a single endpoint.
pub async fn execute_dry_run(config: &BenchmarkConfig) -> Result<DryRunTraceReport> {
    let client = Client::builder()
        .pool_max_idle_per_host(1)
        .tcp_nodelay(true)
        .timeout(config.timeout)
        .redirect(Policy::none())
        .no_proxy()
        .user_agent(crate::client::DEFAULT_USER_AGENT)
        .build()
        .context("failed to initialize dry-run HTTP client")?;

    tokio::time::timeout(DRY_RUN_OVERALL_TIMEOUT, async {
        match config.scenario.as_deref() {
            Some(scenario) => execute_sequence_dry_run(&client, scenario).await,
            None => execute_single_dry_run(&client, config).await,
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("dry-run exceeded 30s execution cap"))?
}

async fn execute_single_dry_run(client: &Client, config: &BenchmarkConfig) -> Result<DryRunTraceReport> {
    let mut request = Request::new(config.method.clone(), config.url.clone());
    if !config.headers.is_empty() {
        *request.headers_mut() = config.headers.clone();
    }
    if let Some(ref body) = config.body {
        *request.body_mut() = Some(Body::from(body.clone()));
    }

    let req_headers = redact_headers(request.headers());
    let resolved_url = sanitize_url(&config.url);
    let start = Instant::now();

    let trace = match client.execute(request).await {
        Ok(mut resp) => {
            let status = resp.status();
            let (body_bytes, body_err) = read_bounded_body(&mut resp, MAX_EXTRACTION_BODY_BYTES).await;
            let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
            let (preview, truncated) = build_preview(&body_bytes, &[]);
            let passed = body_err.is_none() && status.is_success();
            let error = body_err.or_else(|| {
                (!status.is_success()).then(|| format!("received HTTP {}", status.as_u16()))
            });
            DryRunStepTrace {
                step_index: 0,
                order_number: 1,
                step_id: "step_1".to_string(),
                step_name: "Request".to_string(),
                method: config.method.as_str().to_string(),
                resolved_url,
                request_headers: req_headers,
                status_code: Some(status.as_u16()),
                latency_ms,
                bytes_received: body_bytes.len(),
                passed,
                aborted_iteration: !passed,
                error,
                assertions: vec![DryRunAssertionResult {
                    description: "Status 2xx".to_string(),
                    passed: status.is_success(),
                    detail: (!status.is_success())
                        .then(|| format!("received HTTP {}", status.as_u16())),
                }],
                extracted: Vec::new(),
                response_body_preview: preview,
                response_body_truncated: truncated,
            }
        }
        Err(err) => DryRunStepTrace {
            step_index: 0,
            order_number: 1,
            step_id: "step_1".to_string(),
            step_name: "Request".to_string(),
            method: config.method.as_str().to_string(),
            resolved_url,
            request_headers: req_headers,
            status_code: None,
            latency_ms: start.elapsed().as_secs_f64() * 1000.0,
            bytes_received: 0,
            passed: false,
            aborted_iteration: true,
            error: Some(err.to_string()),
            assertions: Vec::new(),
            extracted: Vec::new(),
            response_body_preview: None,
            response_body_truncated: false,
        },
    };

    let completed = trace.passed;
    Ok(DryRunTraceReport {
        completed,
        aborted_at_step: (!completed).then_some(0),
        total_duration_ms: trace.latency_ms,
        steps: vec![trace],
    })
}

async fn execute_sequence_dry_run(
    client: &Client,
    scenario: &CompiledScenario,
) -> Result<DryRunTraceReport> {
    let run_start = Instant::now();
    let mut vars = HashMap::with_capacity(scenario.initial_variables.len() + 8);
    let mut sensitive_values: Vec<String> = Vec::new();

    for (name, value, sensitive) in &scenario.initial_variables {
        vars.insert(name.clone(), value.clone());
        if *sensitive && !value.is_empty() {
            sensitive_values.push(value.clone());
        }
    }

    let built_ins = BuiltInVars {
        vu_id: 0,
        iteration: 1,
        timestamp_ms: current_timestamp_ms(),
    };

    let mut traces = Vec::with_capacity(scenario.steps.len());
    let mut aborted_at_step = None;

    for step in &scenario.steps {
        let trace = execute_single_step_trace(
            client,
            step,
            &mut vars,
            &built_ins,
            &mut sensitive_values,
        )
        .await;
        let should_abort = !trace.passed && step.on_failure == StepFailurePolicy::AbortIteration;
        traces.push(trace);

        if should_abort {
            aborted_at_step = Some(step.step_index);
            break;
        }

        if !step.think_time.is_zero() {
            // Bound dry-run sleep per step so long load think times don't stall UI previews.
            tokio::time::sleep(step.think_time.min(Duration::from_secs(2))).await;
        }
    }

    Ok(DryRunTraceReport {
        completed: aborted_at_step.is_none(),
        aborted_at_step,
        total_duration_ms: run_start.elapsed().as_secs_f64() * 1000.0,
        steps: traces,
    })
}

async fn execute_single_step_trace(
    client: &Client,
    step: &CompiledStep,
    vars: &mut HashMap<String, String>,
    built_ins: &BuiltInVars,
    sensitive_values: &mut Vec<String>,
) -> DryRunStepTrace {
    let (request, resolved_url, raw_headers) = match step.build_request(vars, built_ins) {
        Ok(parts) => parts,
        Err(err) => {
            return DryRunStepTrace {
                step_index: step.step_index,
                order_number: step.order_number,
                step_id: step.id.clone(),
                step_name: step.name.clone(),
                method: step.method.as_str().to_string(),
                resolved_url: step.target_preview.clone(),
                request_headers: HashMap::new(),
                status_code: None,
                latency_ms: 0.0,
                bytes_received: 0,
                passed: false,
                aborted_iteration: step.on_failure == StepFailurePolicy::AbortIteration,
                error: Some(err),
                assertions: Vec::new(),
                extracted: Vec::new(),
                response_body_preview: None,
                response_body_truncated: false,
            };
        }
    };

    let req_headers = redact_headers(&raw_headers);
    let resolved_url = scrub_sensitive_text(&resolved_url, sensitive_values);
    let step_start = Instant::now();

    match client.execute(request).await {
        Ok(mut resp) => {
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let (body_bytes, body_err) = read_bounded_body(&mut resp, MAX_EXTRACTION_BODY_BYTES).await;
            let latency_ms = step_start.elapsed().as_secs_f64() * 1000.0;

            if let Some(err) = body_err {
                let (preview, truncated) = build_preview(&body_bytes, sensitive_values);
                return DryRunStepTrace {
                    step_index: step.step_index,
                    order_number: step.order_number,
                    step_id: step.id.clone(),
                    step_name: step.name.clone(),
                    method: step.method.as_str().to_string(),
                    resolved_url,
                    request_headers: req_headers,
                    status_code: Some(status.as_u16()),
                    latency_ms,
                    bytes_received: body_bytes.len(),
                    passed: false,
                    aborted_iteration: step.on_failure == StepFailurePolicy::AbortIteration,
                    error: Some(err),
                    assertions: Vec::new(),
                    extracted: Vec::new(),
                    response_body_preview: preview,
                    response_body_truncated: truncated,
                };
            }

            let eval = step.evaluate_response(
                status,
                &resp_headers,
                Some(&body_bytes),
                vars,
                true,
            );

            for ex in &eval.extracted {
                if ex.sensitive {
                    if let Some(ref raw_val) = ex.raw_value {
                        if !raw_val.is_empty() {
                            sensitive_values.push(raw_val.clone());
                        }
                    }
                }
            }

            let extracted: Vec<DryRunExtractedVar> = eval
                .extracted
                .into_iter()
                .map(|ex| {
                    let display_val = match ex.raw_value {
                        Some(ref val) if ex.sensitive => format!("[REDACTED (len={})]", val.len()),
                        Some(val) => val,
                        None => String::new(),
                    };
                    DryRunExtractedVar {
                        name: ex.name,
                        source: ex.source,
                        value: display_val,
                        sensitive: ex.sensitive,
                        found: ex.error.is_none(),
                        required: ex.required,
                        error: ex.error,
                    }
                })
                .collect();

            let assertions: Vec<DryRunAssertionResult> = eval
                .assertions
                .into_iter()
                .map(|a| DryRunAssertionResult {
                    description: a.description,
                    passed: a.passed,
                    detail: a.detail,
                })
                .collect();

            let (preview, truncated) = build_preview(&body_bytes, sensitive_values);
            DryRunStepTrace {
                step_index: step.step_index,
                order_number: step.order_number,
                step_id: step.id.clone(),
                step_name: step.name.clone(),
                method: step.method.as_str().to_string(),
                resolved_url,
                request_headers: req_headers,
                status_code: Some(status.as_u16()),
                latency_ms,
                bytes_received: body_bytes.len(),
                passed: eval.passed,
                aborted_iteration: !eval.passed && step.on_failure == StepFailurePolicy::AbortIteration,
                error: eval.error_message,
                assertions,
                extracted,
                response_body_preview: preview,
                response_body_truncated: truncated,
            }
        }
        Err(err) => DryRunStepTrace {
            step_index: step.step_index,
            order_number: step.order_number,
            step_id: step.id.clone(),
            step_name: step.name.clone(),
            method: step.method.as_str().to_string(),
            resolved_url,
            request_headers: req_headers,
            status_code: None,
            latency_ms: step_start.elapsed().as_secs_f64() * 1000.0,
            bytes_received: 0,
            passed: false,
            aborted_iteration: step.on_failure == StepFailurePolicy::AbortIteration,
            error: Some(err.to_string()),
            assertions: Vec::new(),
            extracted: Vec::new(),
            response_body_preview: None,
            response_body_truncated: false,
        },
    }
}

async fn read_bounded_body(
    resp: &mut reqwest::Response,
    max_bytes: usize,
) -> (Vec<u8>, Option<String>) {
    let mut buf = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len().saturating_add(chunk.len()) > max_bytes {
                    let remaining = max_bytes.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..remaining]);
                    return (
                        buf,
                        Some(format!(
                            "BodyExceedsExtractionLimit(max={}B)",
                            max_bytes
                        )),
                    );
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => return (buf, None),
            Err(err) => return (buf, Some(err.to_string())),
        }
    }
}

fn build_preview(bytes: &[u8], sensitive_values: &[String]) -> (Option<String>, bool) {
    if bytes.is_empty() {
        return (None, false);
    }
    let truncated = bytes.len() > DRY_RUN_PREVIEW_LIMIT;
    let slice = if truncated {
        &bytes[..DRY_RUN_PREVIEW_LIMIT]
    } else {
        bytes
    };
    let lossy = String::from_utf8_lossy(slice);
    let scrubbed = scrub_sensitive_text(&lossy, sensitive_values);
    (Some(scrubbed), truncated)
}

/// Formats a [`DryRunTraceReport`] as a terminal-friendly `comfy-table` summary.
pub fn render_dry_run_report(report: &DryRunTraceReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "=== 1-VU Dry-Run Trace ({}) | Total: {:.2} ms ===\n",
        if report.completed { "COMPLETED" } else { "FAILED / ABORTED" },
        report.total_duration_ms
    ));

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("#"),
            Cell::new("Step"),
            Cell::new("Method"),
            Cell::new("Target"),
            Cell::new("Status"),
            Cell::new("Latency"),
            Cell::new("Bytes"),
            Cell::new("Result"),
        ]);

    for step in &report.steps {
        let status = step
            .status_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "ERR".to_string());
        let result = if step.passed {
            "PASS".to_string()
        } else {
            format!(
                "FAIL ({})",
                step.error.as_deref().unwrap_or("unknown error")
            )
        };
        table.add_row(vec![
            Cell::new(step.order_number.to_string()),
            Cell::new(&step.step_name),
            Cell::new(&step.method),
            Cell::new(&step.resolved_url),
            Cell::new(status),
            Cell::new(format!("{:.2} ms", step.latency_ms)),
            Cell::new(step.bytes_received.to_string()),
            Cell::new(result),
        ]);
    }

    out.push_str(&table.to_string());
    out.push('\n');

    for step in &report.steps {
        if !step.extracted.is_empty() {
            out.push_str(&format!(
                "\nStep #{} Extracted Variables:\n",
                step.order_number
            ));
            for var in &step.extracted {
                if var.found {
                    out.push_str(&format!(
                        "  - {} ({}) = {}\n",
                        var.name, var.source, var.value
                    ));
                } else {
                    out.push_str(&format!(
                        "  - {} ({}) = MISSING ({})\n",
                        var.name,
                        var.source,
                        var.error.as_deref().unwrap_or("missing")
                    ));
                }
            }
        }
    }

    out
}
