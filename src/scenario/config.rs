//! Ordered multi-step API sequence input schema, compiled execution types, and redaction helpers.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Body, Method, Request, StatusCode, Url};
use serde::{Deserialize, Serialize};

use crate::cli::HttpMethod;
use crate::config::{is_sensitive_header, sanitize_url, BodyMode, HeaderEntry};
use crate::metrics::event::StepFailureKind;
use crate::scenario::extract::{extract_cookie_from_headers, json_value_to_string, JsonPathExpr};
use crate::scenario::template::{BuiltInVars, TemplateString};

const REDACTED_VALUE: &str = "[REDACTED]";

fn default_schema_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_method() -> HttpMethod {
    HttpMethod::Get
}

/// Execution mode for a benchmark configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestMode {
    #[default]
    Single,
    Sequence,
}

/// Ordered multi-step API scenario configuration shared across CLI files, presets, and UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioConfigInput {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_headers: Vec<HeaderEntry>,
    #[serde(default)]
    pub initial_variables: Vec<ScenarioVariableInput>,
    #[serde(default)]
    pub steps: Vec<ApiStepInput>,
}

impl Default for ScenarioConfigInput {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            name: None,
            base_url: None,
            default_headers: Vec::new(),
            initial_variables: Vec::new(),
            steps: Vec::new(),
        }
    }
}

/// Initial seed variable bound at the start of every VU iteration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioVariableInput {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub sensitive: bool,
}

/// Policy controlling how a VU handles a step failure within an iteration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepFailurePolicy {
    #[default]
    AbortIteration,
    Continue,
}

/// Single step in an ordered API call sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiStepInput {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_method")]
    pub method: HttpMethod,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<HeaderEntry>,
    #[serde(default)]
    pub body_mode: BodyMode,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_file: Option<PathBuf>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub think_time: Option<String>,
    #[serde(default)]
    pub extractors: Vec<StepExtractorInput>,
    #[serde(default)]
    pub assertions: StepAssertionsInput,
    #[serde(default)]
    pub on_failure: StepFailurePolicy,
}

/// Response-to-variable extraction rule configured on a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepExtractorInput {
    pub name: String,
    pub source: ExtractorSource,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub sensitive: bool,
}

/// Source location within an HTTP response used to extract a variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtractorSource {
    JsonPath { path: String },
    Header { name: String },
    Cookie { name: String },
    StatusCode,
}

impl ExtractorSource {
    /// Returns a compact human-readable label describing the extraction source.
    pub fn display_label(&self) -> String {
        match self {
            Self::JsonPath { path } => format!("json:{path}"),
            Self::Header { name } => format!("header:{name}"),
            Self::Cookie { name } => format!("cookie:{name}"),
            Self::StatusCode => "status_code".to_string(),
        }
    }
}

/// Step response assertions evaluated before proceeding to downstream steps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepAssertionsInput {
    #[serde(default)]
    pub status: StatusExpectation,
    #[serde(default)]
    pub json_checks: Vec<JsonCheckInput>,
}

/// Expected HTTP status condition for a sequence step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StatusExpectation {
    #[default]
    #[serde(rename = "any_2xx")]
    Any2xx,
    Exact {
        codes: Vec<u16>,
    },
    Range {
        min: u16,
        max: u16,
    },
    Any,
}

impl StatusExpectation {
    /// Returns `true` when `status_code` satisfies this status expectation.
    pub fn matches(&self, status_code: u16) -> bool {
        match self {
            Self::Any2xx => (200..300).contains(&status_code),
            Self::Exact { codes } => codes.contains(&status_code),
            Self::Range { min, max } => status_code >= *min && status_code <= *max,
            Self::Any => true,
        }
    }

    /// Formats a concise summary string for tables and review panels.
    pub fn summary(&self) -> String {
        match self {
            Self::Any2xx => "2xx".to_string(),
            Self::Exact { codes } => {
                let joined: Vec<String> = codes.iter().map(u16::to_string).collect();
                joined.join(", ")
            }
            Self::Range { min, max } => format!("{min}..={max}"),
            Self::Any => "Any".to_string(),
        }
    }
}

/// Comparison operator for JSON body checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonCheckOp {
    #[default]
    Exists,
    Equals,
}

/// JSON field assertion evaluated against a parsed JSON response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonCheckInput {
    pub path: String,
    #[serde(default)]
    pub op: JsonCheckOp,
    #[serde(default)]
    pub expected: Option<serde_json::Value>,
}

/// Lightweight redacted metadata for one enabled compiled step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioStepMeta {
    pub step_index: usize,
    pub order_number: usize,
    pub id: String,
    pub name: String,
    pub method: String,
    pub target_preview: String,
    pub on_failure: StepFailurePolicy,
}

/// Redacted scenario summary attached to [`MetaResponse`], [`JsonReport`], and [`ExportReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioMetaSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub step_count: usize,
    pub steps: Vec<ScenarioStepMeta>,
}

/// Pre-compiled header template entry.
#[derive(Debug, Clone)]
pub struct CompiledHeader {
    pub name: HeaderName,
    pub value_template: TemplateString,
    pub sensitive: bool,
}

/// Pre-compiled body payload for a step.
#[derive(Debug, Clone)]
pub enum CompiledBody {
    Text(TemplateString),
    Binary(Vec<u8>),
}

/// Pre-compiled variable extractor for runtime execution.
#[derive(Debug, Clone)]
pub enum CompiledExtractorSource {
    JsonPath(JsonPathExpr),
    Header(HeaderName),
    Cookie(String),
    StatusCode,
}

/// Pre-compiled extractor entry attached to a [`CompiledStep`].
#[derive(Debug, Clone)]
pub struct CompiledExtractor {
    pub name: String,
    pub source: CompiledExtractorSource,
    pub source_label: String,
    pub required: bool,
    pub sensitive: bool,
}

/// Pre-compiled JSON assertion attached to a [`CompiledStep`].
#[derive(Debug, Clone)]
pub struct CompiledJsonCheck {
    pub path: JsonPathExpr,
    pub op: JsonCheckOp,
    pub expected: Option<serde_json::Value>,
}

/// Single enabled step compiled for low-overhead worker and dry-run execution.
#[derive(Debug, Clone)]
pub struct CompiledStep {
    pub step_index: usize,
    pub order_number: usize,
    pub id: String,
    pub name: String,
    pub method: Method,
    pub url_template: TemplateString,
    pub target_preview: String,
    pub headers: Vec<CompiledHeader>,
    pub body: Option<CompiledBody>,
    pub timeout: Option<Duration>,
    pub think_time: Duration,
    pub extractors: Vec<CompiledExtractor>,
    pub status_expectation: StatusExpectation,
    pub json_checks: Vec<CompiledJsonCheck>,
    pub on_failure: StepFailurePolicy,
}

/// Fully compiled ordered sequence attached to a [`BenchmarkConfig`].
#[derive(Debug, Clone)]
pub struct CompiledScenario {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub initial_variables: Vec<(String, String, bool)>,
    pub steps: Vec<CompiledStep>,
    pub unique_host_count: usize,
    pub sensitive_var_names: HashSet<String>,
}

/// Diagnostic outcome of evaluating a single assertion.
#[derive(Debug, Clone)]
pub struct AssertionOutcome {
    pub description: String,
    pub passed: bool,
    pub detail: Option<String>,
}

/// Diagnostic outcome of executing a single extractor.
#[derive(Debug, Clone)]
pub struct ExtractedVarOutcome {
    pub name: String,
    pub source: String,
    pub raw_value: Option<String>,
    pub sensitive: bool,
    pub required: bool,
    pub error: Option<String>,
}

/// Combined outcome of evaluating assertions and extractors on a step HTTP response.
#[derive(Debug, Clone)]
pub struct StepEvaluationResult {
    pub passed: bool,
    pub failure_kind: Option<StepFailureKind>,
    pub error_message: Option<String>,
    pub assertions: Vec<AssertionOutcome>,
    pub extracted: Vec<ExtractedVarOutcome>,
}

impl CompiledStep {
    /// Returns `true` when this step inspects the response body for JSON checks or extraction.
    pub fn needs_body_buffer(&self) -> bool {
        !self.json_checks.is_empty()
            || self
                .extractors
                .iter()
                .any(|ex| matches!(ex.source, CompiledExtractorSource::JsonPath(_)))
    }

    /// Builds a [`reqwest::Request`] by resolving URL, headers, and body against `vars` and `built_ins`.
    pub fn build_request(
        &self,
        vars: &HashMap<String, String>,
        built_ins: &BuiltInVars,
    ) -> Result<(Request, String, HeaderMap), String> {
        let rendered_url = self.url_template.render(vars, built_ins);
        let parsed_url = Url::parse(&rendered_url)
            .map_err(|err| format!("failed to build URL '{rendered_url}': {err}"))?;
        let sanitized_url = sanitize_url(&parsed_url);

        let mut request = Request::new(self.method.clone(), parsed_url);
        let mut resolved_headers = HeaderMap::with_capacity(self.headers.len());

        for hdr in &self.headers {
            let rendered_val = hdr.value_template.render(vars, built_ins);
            let val = HeaderValue::from_str(rendered_val.trim()).map_err(|err| {
                format!(
                    "invalid rendered value for header '{}': {err}",
                    hdr.name.as_str()
                )
            })?;
            resolved_headers.append(hdr.name.clone(), val);
        }

        if !resolved_headers.is_empty() {
            *request.headers_mut() = resolved_headers.clone();
        }

        if let Some(ref body_tpl) = self.body {
            match body_tpl {
                CompiledBody::Text(tpl) => {
                    let rendered_body = tpl.render(vars, built_ins);
                    *request.body_mut() = Some(Body::from(rendered_body));
                }
                CompiledBody::Binary(bytes) => {
                    *request.body_mut() = Some(Body::from(bytes.clone()));
                }
            }
        }

        if let Some(step_timeout) = self.timeout {
            *request.timeout_mut() = Some(step_timeout);
        }

        Ok((request, sanitized_url, resolved_headers))
    }

    /// Evaluates status expectations, JSON checks, and response extractors against a response.
    ///
    /// When `collect_diagnostics` is `false` (worker hot path), assertion/extractor diagnostic
    /// vectors are left empty to avoid unnecessary per-request allocations.
    pub fn evaluate_response(
        &self,
        status: StatusCode,
        resp_headers: &HeaderMap,
        body_bytes: Option<&[u8]>,
        vars: &mut HashMap<String, String>,
        collect_diagnostics: bool,
    ) -> StepEvaluationResult {
        let code = status.as_u16();
        let status_ok = self.status_expectation.matches(code);

        let mut assertions = if collect_diagnostics {
            vec![AssertionOutcome {
                description: format!("Status {}", self.status_expectation.summary()),
                passed: status_ok,
                detail: (!status_ok).then(|| {
                    format!(
                        "received HTTP {code}, expected {}",
                        self.status_expectation.summary()
                    )
                }),
            }]
        } else {
            Vec::new()
        };

        let mut failure_kind: Option<StepFailureKind> = None;
        let mut error_message: Option<String> = None;

        if !status_ok {
            failure_kind = Some(StepFailureKind::AssertionFailed);
            error_message = Some(format!(
                "StatusMismatch(got={code},expected={})",
                self.status_expectation.summary()
            ));
        }

        let needs_json = !self.json_checks.is_empty()
            || self
                .extractors
                .iter()
                .any(|ex| matches!(ex.source, CompiledExtractorSource::JsonPath(_)));

        let parsed_json: Option<Result<serde_json::Value, String>> = if needs_json {
            match body_bytes {
                Some(bytes) => Some(
                    serde_json::from_slice::<serde_json::Value>(bytes)
                        .map_err(|err| format!("invalid JSON response body: {err}")),
                ),
                None => Some(Err("response body was not buffered".to_string())),
            }
        } else {
            None
        };

        for check in &self.json_checks {
            let (check_ok, detail) = match parsed_json.as_ref() {
                Some(Ok(doc)) => match check.path.resolve(doc) {
                    None => (
                        false,
                        Some(format!("JSON path '{}' not found", check.path.raw)),
                    ),
                    Some(found) => match check.op {
                        JsonCheckOp::Exists => (true, None),
                        JsonCheckOp::Equals => {
                            let expected = check.expected.as_ref().unwrap_or(&serde_json::Value::Null);
                            if found == expected {
                                (true, None)
                            } else {
                                (
                                    false,
                                    Some(format!(
                                        "JSON '{}' was {}, expected {}",
                                        check.path.raw, found, expected
                                    )),
                                )
                            }
                        }
                    },
                },
                Some(Err(err)) => (false, Some(err.clone())),
                None => (false, Some("missing JSON body".to_string())),
            };

            if !check_ok && failure_kind.is_none() {
                failure_kind = Some(StepFailureKind::AssertionFailed);
                error_message = Some("JsonAssertionFailed".to_string());
            }

            if collect_diagnostics {
                let desc = match check.op {
                    JsonCheckOp::Exists => format!("JSON {} exists", check.path.raw),
                    JsonCheckOp::Equals => format!(
                        "JSON {} == {}",
                        check.path.raw,
                        check
                            .expected
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "null".to_string())
                    ),
                };
                assertions.push(AssertionOutcome {
                    description: desc,
                    passed: check_ok,
                    detail,
                });
            }
        }

        let mut extracted = if collect_diagnostics {
            Vec::with_capacity(self.extractors.len())
        } else {
            Vec::new()
        };

        for ex in &self.extractors {
            let extracted_res: Result<String, String> = match &ex.source {
                CompiledExtractorSource::StatusCode => Ok(code.to_string()),
                CompiledExtractorSource::Header(header_name) => resp_headers
                    .get(header_name)
                    .ok_or_else(|| format!("response header '{}' missing", header_name.as_str()))
                    .and_then(|hv| {
                        hv.to_str()
                            .map(|s| s.to_string())
                            .map_err(|_| format!("header '{}' is not valid UTF-8", header_name.as_str()))
                    }),
                CompiledExtractorSource::Cookie(cookie_name) => {
                    extract_cookie_from_headers(resp_headers, cookie_name)
                        .ok_or_else(|| format!("Set-Cookie '{}' not found", cookie_name))
                }
                CompiledExtractorSource::JsonPath(expr) => match parsed_json.as_ref() {
                    Some(Ok(doc)) => match expr.resolve(doc) {
                        Some(val) => json_value_to_string(val),
                        None => Err(format!("JSON path '{}' not found", expr.raw)),
                    },
                    Some(Err(err)) => Err(err.clone()),
                    None => Err("JSON response body unavailable".to_string()),
                },
            };

            match extracted_res {
                Ok(val) => {
                    vars.insert(ex.name.clone(), val.clone());
                    if collect_diagnostics {
                        extracted.push(ExtractedVarOutcome {
                            name: ex.name.clone(),
                            source: ex.source_label.clone(),
                            raw_value: Some(val),
                            sensitive: ex.sensitive,
                            required: ex.required,
                            error: None,
                        });
                    }
                }
                Err(err) => {
                    if ex.required && failure_kind.is_none() {
                        failure_kind = Some(StepFailureKind::ExtractionFailed);
                        error_message = Some(format!("ExtractionFailed({})", ex.name));
                    }
                    if collect_diagnostics {
                        extracted.push(ExtractedVarOutcome {
                            name: ex.name.clone(),
                            source: ex.source_label.clone(),
                            raw_value: None,
                            sensitive: ex.sensitive,
                            required: ex.required,
                            error: Some(err),
                        });
                    }
                }
            }
        }

        StepEvaluationResult {
            passed: failure_kind.is_none(),
            failure_kind,
            error_message,
            assertions,
            extracted,
        }
    }
}

impl CompiledScenario {
    /// Returns redacted metadata descriptors for all compiled steps in execution order.
    pub fn step_metas(&self) -> Vec<ScenarioStepMeta> {
        self.steps
            .iter()
            .map(|step| ScenarioStepMeta {
                step_index: step.step_index,
                order_number: step.order_number,
                id: step.id.clone(),
                name: step.name.clone(),
                method: step.method.as_str().to_string(),
                target_preview: step.target_preview.clone(),
                on_failure: step.on_failure,
            })
            .collect()
    }

    /// Builds a redacted [`ScenarioMetaSummary`] suitable for API metadata and exports.
    pub fn meta_summary(&self) -> ScenarioMetaSummary {
        let steps = self.step_metas();
        ScenarioMetaSummary {
            name: self.name.clone(),
            base_url: self
                .base_url
                .as_deref()
                .map(sanitize_url_string_preserving_templates),
            step_count: steps.len(),
            steps,
        }
    }
}

/// Returns `true` if a variable name is likely to carry credentials, tokens, or secrets.
pub fn is_sensitive_var_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("passwd")
        || lower.contains("pwd")
        || lower.contains("key")
        || lower.contains("auth")
        || lower.contains("cookie")
        || lower.contains("session")
        || lower.contains("jwt")
        || lower.contains("bearer")
        || lower.contains("credential")
        || lower.contains("csrf")
}

/// Redacts embedded credentials from a URL string while leaving `{{var}}` placeholders intact.
pub fn sanitize_url_string_preserving_templates(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(parsed) = Url::parse(trimmed) {
        return sanitize_url(&parsed);
    }
    // Mask `user:pass@` before the first path/query delimiter if templates prevented Url::parse.
    if let Some(scheme_idx) = trimmed.find("://") {
        let after_scheme = &trimmed[scheme_idx + 3..];
        let auth_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..auth_end];
        if let Some(at_idx) = authority.rfind('@') {
            let prefix = &trimmed[..scheme_idx + 3];
            let host_and_rest = &after_scheme[at_idx + 1..];
            return format!("{prefix}REDACTED:REDACTED@{host_and_rest}");
        }
    }
    trimmed.to_string()
}

/// Returns a cloned [`ScenarioConfigInput`] with sensitive headers, URL userinfo, and variables masked.
pub fn redact_scenario_input(scenario: &ScenarioConfigInput) -> ScenarioConfigInput {
    let mut out = scenario.clone();
    out.base_url = out
        .base_url
        .as_deref()
        .map(sanitize_url_string_preserving_templates);

    for header in &mut out.default_headers {
        if is_sensitive_header(&header.name) && !header.value.is_empty() {
            header.value = REDACTED_VALUE.to_string();
        }
    }

    for var in &mut out.initial_variables {
        if (var.sensitive || is_sensitive_var_name(&var.name)) && !var.value.is_empty() {
            var.value = REDACTED_VALUE.to_string();
        }
    }

    for step in &mut out.steps {
        step.url = sanitize_url_string_preserving_templates(&step.url);
        for header in &mut step.headers {
            if is_sensitive_header(&header.name) && !header.value.is_empty() {
                header.value = REDACTED_VALUE.to_string();
            }
        }
    }

    out
}

/// Replaces literal occurrences of sensitive values (`len >= 4`) inside `text` with `"[REDACTED]"`.
pub fn scrub_sensitive_text(text: &str, sensitive_values: &[String]) -> String {
    let mut result = text.to_string();
    let mut secrets: Vec<&str> = sensitive_values
        .iter()
        .map(String::as_str)
        .filter(|s| s.len() >= 4)
        .collect();
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets.dedup();

    for secret in secrets {
        if result.contains(secret) {
            result = result.replace(secret, REDACTED_VALUE);
        }
    }
    result
}
