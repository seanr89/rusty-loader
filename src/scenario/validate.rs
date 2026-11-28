//! Static validation, variable lineage analysis, review formatting, and compilation for scenarios.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Duration;

use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Table};
use reqwest::header::HeaderName;
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};

use crate::cli::{parse_duration, parse_timeout};
use crate::config::{
    validate_header_entry, validate_target_url, BodyMode, ConfigError, HeaderEntry,
    ValidationReport,
};
use crate::os::OsLimits;
use crate::scenario::config::{
    is_sensitive_var_name, sanitize_url_string_preserving_templates, ApiStepInput, CompiledBody,
    CompiledExtractor, CompiledExtractorSource, CompiledHeader, CompiledJsonCheck,
    CompiledScenario, CompiledStep, ExtractorSource, JsonCheckOp, ScenarioConfigInput,
    StatusExpectation, StepFailurePolicy,
};
use crate::scenario::extract::parse_json_path;
use crate::scenario::template::{is_builtin_var, is_valid_var_name, parse_template, TemplateString};

/// Single step entry inside a pre-run [`ScenarioReviewReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepReviewItem {
    pub step_index: usize,
    pub order_number: usize,
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub method: String,
    pub target_preview: String,
    pub consumes: Vec<String>,
    pub produces: Vec<String>,
    pub assertions_summary: String,
    pub on_failure: StepFailurePolicy,
    pub think_time_ms: u64,
    pub timeout_ms: Option<u64>,
    pub issues: Vec<String>,
}

/// Complete pre-run static review report produced for `--review` and the UI Sequence Studio.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioReviewReport {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub enabled_step_count: usize,
    pub total_step_count: usize,
    pub unique_hosts: usize,
    pub initial_variables: Vec<String>,
    pub steps: Vec<StepReviewItem>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

fn origin_key(url: &Url) -> String {
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");
    let port = url.port_or_known_default().unwrap_or(0);
    format!("{scheme}://{host}:{port}")
}

fn combine_step_url_template(
    base_url: Option<&str>,
    raw_step_url: &str,
) -> Result<(String, bool), String> {
    let trimmed = raw_step_url.trim();
    if trimmed.is_empty() {
        return Err("step URL or path cannot be empty".to_string());
    }

    if let Some(scheme_end) = trimmed.find("://") {
        let before_scheme = &trimmed[..scheme_end];
        if !before_scheme.contains('/') && !before_scheme.contains('?') && !before_scheme.contains('#') {
            if before_scheme.contains("{{") {
                return Err("URL scheme cannot contain template variables".to_string());
            }
            return Ok((trimmed.to_string(), false));
        }
    }

    let Some(base) = base_url.map(str::trim).filter(|b| !b.is_empty()) else {
        return Err(
            "relative step path requires a Scenario Base URL (or enter a full http(s):// URL)"
                .to_string(),
        );
    };

    let base_trimmed = base.trim_end_matches('/');
    let combined = if trimmed.starts_with('/') || trimmed.starts_with('?') || trimmed.starts_with('#')
    {
        format!("{base_trimmed}{trimmed}")
    } else {
        format!("{base_trimmed}/{trimmed}")
    };
    Ok((combined, true))
}

fn merge_step_headers(
    default_headers: &[HeaderEntry],
    step_headers: &[HeaderEntry],
) -> Vec<HeaderEntry> {
    let overridden: HashSet<String> = step_headers
        .iter()
        .filter(|h| h.enabled && !h.name.trim().is_empty())
        .map(|h| h.name.trim().to_ascii_lowercase())
        .collect();

    let mut merged = Vec::with_capacity(default_headers.len() + step_headers.len());
    for entry in default_headers {
        if !entry.enabled {
            continue;
        }
        if !overridden.contains(&entry.name.trim().to_ascii_lowercase()) {
            merged.push(entry.clone());
        }
    }
    for entry in step_headers {
        if entry.enabled {
            merged.push(entry.clone());
        }
    }
    merged
}

fn format_assertions_summary(step: &ApiStepInput) -> String {
    let status_part = format!("HTTP {}", step.assertions.status.summary());
    if step.assertions.json_checks.is_empty() {
        status_part
    } else {
        format!("{status_part} + {} JSON", step.assertions.json_checks.len())
    }
}

/// Validates a [`ScenarioConfigInput`] statically and returns both structured form errors and a [`ScenarioReviewReport`].
pub fn validate_scenario(
    input: &ScenarioConfigInput,
    default_timeout: Duration,
    concurrency: usize,
    os_limits: Option<OsLimits>,
) -> (ValidationReport, ScenarioReviewReport) {
    let mut field_errors: HashMap<String, String> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut global_errors: Vec<String> = Vec::new();

    let mut record_error = |field: String, msg: String| {
        if !global_errors.contains(&msg) {
            global_errors.push(msg.clone());
        }
        field_errors.entry(field).or_insert(msg);
    };

    let normalized_base_url = input
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(base) = normalized_base_url {
        if let Err(err) = validate_target_url(base) {
            record_error("scenario.base_url".to_string(), err.to_string());
        }
    }

    for (idx, hdr) in input.default_headers.iter().enumerate() {
        if !hdr.enabled {
            continue;
        }
        if let Err(err) = parse_template(&hdr.value) {
            record_error(
                format!("scenario.default_headers.{idx}"),
                format!("header '{}' value template error: {err}", hdr.name.trim()),
            );
            continue;
        }
        let dummy_val = parse_template(&hdr.value)
            .map(|t| t.render_with_dummy())
            .unwrap_or_else(|_| hdr.value.clone());
        if let Err(err) = validate_header_entry(&hdr.name, &dummy_val) {
            record_error(format!("scenario.default_headers.{idx}"), err.to_string());
        }
    }

    let mut available_vars: HashSet<String> = HashSet::new();
    let mut initial_var_names: Vec<String> = Vec::new();
    let mut seen_initial: HashSet<String> = HashSet::new();

    for (idx, var) in input.initial_variables.iter().enumerate() {
        let name = var.name.trim();
        if name.is_empty() {
            record_error(
                format!("scenario.initial_variables.{idx}.name"),
                "initial variable name cannot be empty".to_string(),
            );
            continue;
        }
        if is_builtin_var(name) {
            record_error(
                format!("scenario.initial_variables.{idx}.name"),
                format!("variable name '{name}' is reserved for built-in VU runtime variables"),
            );
            continue;
        }
        if !is_valid_var_name(name) {
            record_error(
                format!("scenario.initial_variables.{idx}.name"),
                format!("invalid variable name '{name}': expected [A-Za-z_][A-Za-z0-9_]*"),
            );
            continue;
        }
        if !seen_initial.insert(name.to_string()) {
            record_error(
                format!("scenario.initial_variables.{idx}.name"),
                format!("duplicate initial variable '{name}'"),
            );
            continue;
        }
        available_vars.insert(name.to_string());
        initial_var_names.push(name.to_string());
    }

    // Track disabled producers so out-of-order/disabled references produce clearer guidance.
    let mut all_producer_indices: HashMap<String, (usize, bool)> = HashMap::new();
    for (idx, step) in input.steps.iter().enumerate() {
        for ex in &step.extractors {
            let ex_name = ex.name.trim();
            if !ex_name.is_empty() {
                all_producer_indices
                    .entry(ex_name.to_string())
                    .or_insert((idx, step.enabled));
            }
        }
    }

    let mut seen_step_ids: HashSet<String> = HashSet::new();
    let mut enabled_step_count = 0usize;
    let mut unique_origins: HashSet<String> = HashSet::new();
    let mut extracted_usage: HashMap<String, (usize, bool)> = HashMap::new();
    let mut step_reviews: Vec<StepReviewItem> = Vec::with_capacity(input.steps.len());

    for (raw_idx, step) in input.steps.iter().enumerate() {
        let order_number = raw_idx + 1;
        let step_prefix = format!("scenario.steps.{raw_idx}");
        let mut step_issues: Vec<String> = Vec::new();

        let id_trimmed = step.id.trim();
        if id_trimmed.is_empty() {
            let msg = format!("step #{order_number} id cannot be empty");
            step_issues.push(msg.clone());
            record_error(format!("{step_prefix}.id"), msg);
        } else if !seen_step_ids.insert(id_trimmed.to_string()) {
            let msg = format!("duplicate step id '{id_trimmed}'");
            step_issues.push(msg.clone());
            record_error(format!("{step_prefix}.id"), msg);
        }

        if step.name.trim().is_empty() {
            let msg = format!("step #{order_number} requires a name");
            step_issues.push(msg.clone());
            record_error(format!("{step_prefix}.name"), msg);
        }

        let mut consumes: Vec<String> = Vec::new();
        let mut consumes_seen: HashSet<String> = HashSet::new();
        let mut produces: Vec<String> = Vec::new();
        let mut target_preview = sanitize_url_string_preserving_templates(&step.url);

        if !step.enabled {
            warnings.push(format!(
                "Step #{order_number} ('{}') is disabled and will be skipped.",
                step.name
            ));
            step_reviews.push(StepReviewItem {
                step_index: raw_idx,
                order_number,
                id: step.id.clone(),
                name: step.name.clone(),
                enabled: false,
                method: step.method.to_string(),
                target_preview,
                consumes,
                produces,
                assertions_summary: format_assertions_summary(step),
                on_failure: step.on_failure,
                think_time_ms: 0,
                timeout_ms: None,
                issues: step_issues,
            });
            continue;
        }

        let compiled_step_index = enabled_step_count;
        enabled_step_count += 1;

        // 1. Validate URL template and combined URL
        match combine_step_url_template(normalized_base_url, &step.url) {
            Ok((combined_url, _is_rel)) => {
                target_preview = sanitize_url_string_preserving_templates(&combined_url);
                match parse_template(&combined_url) {
                    Ok(tpl) => {
                        for var_name in tpl.referenced_vars() {
                            if consumes_seen.insert(var_name.clone()) {
                                consumes.push(var_name.clone());
                            }
                        }
                        let dummy_url = tpl.render_with_dummy();
                        match validate_target_url(&dummy_url) {
                            Ok(parsed) => {
                                unique_origins.insert(origin_key(&parsed));
                            }
                            Err(err) => {
                                let msg = err.to_string();
                                step_issues.push(msg.clone());
                                record_error(format!("{step_prefix}.url"), msg);
                            }
                        }
                    }
                    Err(err) => {
                        let msg = format!("step #{order_number} URL template error: {err}");
                        step_issues.push(msg.clone());
                        record_error(format!("{step_prefix}.url"), msg);
                    }
                }
            }
            Err(msg) => {
                step_issues.push(msg.clone());
                record_error(format!("{step_prefix}.url"), msg);
            }
        }

        // 2. Validate step headers + merged default header templates
        for (h_idx, hdr) in step.headers.iter().enumerate() {
            if !hdr.enabled {
                continue;
            }
            match parse_template(&hdr.value) {
                Ok(tpl) => {
                    for var_name in tpl.referenced_vars() {
                        if consumes_seen.insert(var_name.clone()) {
                            consumes.push(var_name.clone());
                        }
                    }
                    let dummy_val = tpl.render_with_dummy();
                    if let Err(err) = validate_header_entry(&hdr.name, &dummy_val) {
                        let msg = err.to_string();
                        step_issues.push(msg.clone());
                        record_error(format!("{step_prefix}.headers.{h_idx}"), msg);
                    }
                }
                Err(err) => {
                    let msg = format!("header '{}' template error: {err}", hdr.name.trim());
                    step_issues.push(msg.clone());
                    record_error(format!("{step_prefix}.headers.{h_idx}"), msg);
                }
            }
        }

        for hdr in merge_step_headers(&input.default_headers, &step.headers) {
            if let Ok(tpl) = parse_template(&hdr.value) {
                for var_name in tpl.referenced_vars() {
                    if consumes_seen.insert(var_name.clone()) {
                        consumes.push(var_name.clone());
                    }
                }
            }
        }

        // 3. Validate body template / file
        match step.body_mode {
            BodyMode::None => {}
            BodyMode::Raw => {
                if let Some(ref raw_body) = step.body {
                    match parse_template(raw_body) {
                        Ok(tpl) => {
                            for var_name in tpl.referenced_vars() {
                                if consumes_seen.insert(var_name.clone()) {
                                    consumes.push(var_name.clone());
                                }
                            }
                        }
                        Err(err) => {
                            let msg = format!("body template error: {err}");
                            step_issues.push(msg.clone());
                            record_error(format!("{step_prefix}.body"), msg);
                        }
                    }
                }
            }
            BodyMode::File => match step.body_file.as_ref() {
                None => {
                    let msg = "select a request payload file".to_string();
                    step_issues.push(msg.clone());
                    record_error(format!("{step_prefix}.body_file"), msg);
                }
                Some(path) => {
                    let path_str = path.to_string_lossy();
                    let trimmed = path_str.trim();
                    if trimmed.is_empty() {
                        let msg = "request body file path cannot be empty".to_string();
                        step_issues.push(msg.clone());
                        record_error(format!("{step_prefix}.body_file"), msg);
                    } else {
                        match fs::read(trimmed) {
                            Ok(bytes) => {
                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                    match parse_template(text) {
                                        Ok(tpl) => {
                                            for var_name in tpl.referenced_vars() {
                                                if consumes_seen.insert(var_name.clone()) {
                                                    consumes.push(var_name.clone());
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            let msg = format!(
                                                "body file '{trimmed}' template error: {err}"
                                            );
                                            step_issues.push(msg.clone());
                                            record_error(format!("{step_prefix}.body_file"), msg);
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                let msg = format!("cannot read payload file '{trimmed}': {err}");
                                step_issues.push(msg.clone());
                                record_error(format!("{step_prefix}.body_file"), msg);
                            }
                        }
                    }
                }
            },
        }

        // 4. Validate timeout override & think time
        let mut timeout_ms = Some(default_timeout.as_millis() as u64);
        if let Some(ref raw_timeout) = step.timeout {
            let trimmed = raw_timeout.trim();
            if !trimmed.is_empty() {
                match parse_timeout(trimmed) {
                    Ok(dur) if dur.is_zero() => {
                        let msg = "step timeout must be greater than 0ms".to_string();
                        step_issues.push(msg.clone());
                        record_error(format!("{step_prefix}.timeout"), msg);
                    }
                    Ok(dur) => {
                        timeout_ms = Some(dur.as_millis() as u64);
                    }
                    Err(err) => {
                        step_issues.push(err.clone());
                        record_error(format!("{step_prefix}.timeout"), err);
                    }
                }
            }
        }

        let mut think_time_ms = 0u64;
        if let Some(ref raw_think) = step.think_time {
            let trimmed = raw_think.trim();
            if !trimmed.is_empty() {
                match parse_duration(trimmed) {
                    Ok(dur) => {
                        think_time_ms = dur.as_millis() as u64;
                    }
                    Err(err) => {
                        step_issues.push(err.clone());
                        record_error(format!("{step_prefix}.think_time"), err);
                    }
                }
            }
        }

        // 5. Verify consumed variables are already available at this step index
        for var_name in &consumes {
            if is_builtin_var(var_name) {
                continue;
            }
            if available_vars.contains(var_name) {
                if let Some((_, used)) = extracted_usage.get_mut(var_name) {
                    *used = true;
                }
                continue;
            }

            let detail = match all_producer_indices.get(var_name) {
                Some((producer_idx, false)) => format!(
                    "variable '{var_name}' in step #{order_number} ('{}') is extracted by disabled step #{}",
                    step.name,
                    producer_idx + 1
                ),
                Some((producer_idx, true)) if *producer_idx >= raw_idx => format!(
                    "variable '{var_name}' is used in step #{order_number} ('{}') before it is extracted by step #{}",
                    step.name,
                    producer_idx + 1
                ),
                _ => format!(
                    "variable '{var_name}' referenced in step #{order_number} ('{}') is not defined in initial variables or prior steps",
                    step.name
                ),
            };
            step_issues.push(detail.clone());
            record_error(format!("{step_prefix}.url"), detail);
        }

        // 6. Validate assertions
        match &step.assertions.status {
            StatusExpectation::Exact { codes } if codes.is_empty() => {
                let msg = "exact status assertion requires at least one status code".to_string();
                step_issues.push(msg.clone());
                record_error(format!("{step_prefix}.assertions.status"), msg);
            }
            StatusExpectation::Exact { codes } => {
                if codes.iter().any(|c| *c < 100 || *c > 599) {
                    let msg = "HTTP status codes must be between 100 and 599".to_string();
                    step_issues.push(msg.clone());
                    record_error(format!("{step_prefix}.assertions.status"), msg);
                }
            }
            StatusExpectation::Range { min, max } if min > max || *min < 100 || *max > 599 => {
                let msg = format!("invalid status range {min}..={max}");
                step_issues.push(msg.clone());
                record_error(format!("{step_prefix}.assertions.status"), msg);
            }
            _ => {}
        }

        for (c_idx, check) in step.assertions.json_checks.iter().enumerate() {
            if let Err(err) = parse_json_path(&check.path) {
                step_issues.push(err.clone());
                record_error(format!("{step_prefix}.assertions.json_checks.{c_idx}"), err);
            }
            if check.op == JsonCheckOp::Equals && check.expected.is_none() {
                let msg = format!(
                    "JSON equality assertion '{}' requires an expected value",
                    check.path
                );
                step_issues.push(msg.clone());
                record_error(format!("{step_prefix}.assertions.json_checks.{c_idx}"), msg);
            }
        }

        // 7. Validate extractors & register produced variables into downstream scope
        for (e_idx, ex) in step.extractors.iter().enumerate() {
            let var_name = ex.name.trim();
            if var_name.is_empty() {
                let msg = format!("extractor #{} variable name cannot be empty", e_idx + 1);
                step_issues.push(msg.clone());
                record_error(format!("{step_prefix}.extractors.{e_idx}.name"), msg);
                continue;
            }
            if is_builtin_var(var_name) {
                let msg = format!(
                    "extractor variable '{var_name}' cannot overwrite reserved built-in variable"
                );
                step_issues.push(msg.clone());
                record_error(format!("{step_prefix}.extractors.{e_idx}.name"), msg);
                continue;
            }
            if !is_valid_var_name(var_name) {
                let msg = format!(
                    "invalid extractor variable '{var_name}': expected [A-Za-z_][A-Za-z0-9_]*"
                );
                step_issues.push(msg.clone());
                record_error(format!("{step_prefix}.extractors.{e_idx}.name"), msg);
                continue;
            }

            match &ex.source {
                ExtractorSource::JsonPath { path } => {
                    if let Err(err) = parse_json_path(path) {
                        step_issues.push(err.clone());
                        record_error(format!("{step_prefix}.extractors.{e_idx}.source"), err);
                    }
                }
                ExtractorSource::Header { name } => {
                    let h = name.trim();
                    if h.is_empty() || HeaderName::from_bytes(h.as_bytes()).is_err() {
                        let msg = format!("invalid response header name '{name}'");
                        step_issues.push(msg.clone());
                        record_error(format!("{step_prefix}.extractors.{e_idx}.source"), msg);
                    }
                }
                ExtractorSource::Cookie { name } => {
                    if name.trim().is_empty() {
                        let msg = "cookie name cannot be empty".to_string();
                        step_issues.push(msg.clone());
                        record_error(format!("{step_prefix}.extractors.{e_idx}.source"), msg);
                    }
                }
                ExtractorSource::StatusCode => {}
            }

            if !available_vars.insert(var_name.to_string()) {
                warnings.push(format!(
                    "Step #{order_number} ('{}') overwrites existing variable '{var_name}'.",
                    step.name
                ));
            }
            produces.push(var_name.to_string());
            extracted_usage.insert(var_name.to_string(), (order_number, false));
        }

        step_reviews.push(StepReviewItem {
            step_index: compiled_step_index,
            order_number,
            id: step.id.clone(),
            name: step.name.clone(),
            enabled: true,
            method: step.method.to_string(),
            target_preview,
            consumes,
            produces,
            assertions_summary: format_assertions_summary(step),
            on_failure: step.on_failure,
            think_time_ms,
            timeout_ms,
            issues: step_issues,
        });
    }

    if enabled_step_count == 0 {
        record_error(
            "scenario.steps".to_string(),
            "add and enable at least 1 API step in the sequence".to_string(),
        );
    }

    let mut unused_vars: Vec<(&String, usize)> = extracted_usage
        .iter()
        .filter_map(|(name, (order_no, used))| (!*used).then_some((name, *order_no)))
        .collect();
    unused_vars.sort_by_key(|(name, order_no)| (*order_no, name.as_str()));
    for (var_name, order_no) in unused_vars {
        warnings.push(format!(
            "Extracted variable '{var_name}' (Step #{order_no}) is not referenced by any downstream step."
        ));
    }

    let unique_hosts = unique_origins.len().max(1);
    if let Some(limits) = os_limits {
        let estimated_fds = (concurrency as u64)
            .saturating_mul(unique_hosts as u64)
            .saturating_mul(2)
            .saturating_add(128);
        if !limits.is_sufficient() || estimated_fds > limits.soft_limit {
            warnings.push(format!(
                "OS open file limit is {} (concurrency {} across {} host(s) needs ~{} descriptors, recommended >= {}). Run 'ulimit -n 65536' if running from a terminal.",
                limits.soft_limit,
                concurrency,
                unique_hosts,
                estimated_fds,
                limits.recommended_min
            ));
        }
    }

    let valid = field_errors.is_empty();
    let review = ScenarioReviewReport {
        valid,
        name: input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        base_url: normalized_base_url.map(sanitize_url_string_preserving_templates),
        enabled_step_count,
        total_step_count: input.steps.len(),
        unique_hosts,
        initial_variables: initial_var_names,
        steps: step_reviews,
        warnings: warnings.clone(),
        errors: global_errors,
    };

    let report = ValidationReport {
        valid,
        field_errors,
        warnings,
    };

    (report, review)
}

/// Validates and compiles a [`ScenarioConfigInput`] into an executable [`CompiledScenario`].
pub fn compile_scenario(
    input: &ScenarioConfigInput,
    default_timeout: Duration,
) -> Result<CompiledScenario, ConfigError> {
    let (report, _) = validate_scenario(input, default_timeout, 1, None);
    if !report.valid {
        let first_err = report
            .field_errors
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| "invalid API sequence configuration".to_string());
        return Err(ConfigError::InvalidScenario(first_err));
    }

    let normalized_base = input
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut sensitive_var_names = HashSet::new();
    let mut initial_variables = Vec::with_capacity(input.initial_variables.len());
    for var in &input.initial_variables {
        let name = var.name.trim().to_string();
        let sensitive = var.sensitive || is_sensitive_var_name(&name);
        if sensitive {
            sensitive_var_names.insert(name.clone());
        }
        initial_variables.push((name, var.value.clone(), sensitive));
    }

    let mut unique_origins = HashSet::new();
    let mut compiled_steps = Vec::new();

    for (raw_idx, step) in input.steps.iter().enumerate() {
        if !step.enabled {
            continue;
        }

        let (combined_url, _) = combine_step_url_template(normalized_base.as_deref(), &step.url)
            .map_err(ConfigError::InvalidScenario)?;
        let url_template = TemplateString::parse(&combined_url).map_err(ConfigError::InvalidScenario)?;
        if let Ok(parsed) = Url::parse(&url_template.render_with_dummy()) {
            unique_origins.insert(origin_key(&parsed));
        }

        let merged_headers = merge_step_headers(&input.default_headers, &step.headers);
        let mut compiled_headers = Vec::with_capacity(merged_headers.len());
        for entry in merged_headers {
            let name_trimmed = entry.name.trim();
            let header_name = HeaderName::from_bytes(name_trimmed.as_bytes()).map_err(|err| {
                ConfigError::InvalidHeader(format!("invalid header name '{name_trimmed}': {err}"))
            })?;
            let value_template =
                TemplateString::parse(entry.value.trim()).map_err(ConfigError::InvalidScenario)?;
            compiled_headers.push(CompiledHeader {
                name: header_name,
                value_template,
                sensitive: crate::config::is_sensitive_header(name_trimmed),
            });
        }

        let compiled_body = match step.body_mode {
            BodyMode::None => None,
            BodyMode::Raw => match step.body.as_ref() {
                Some(raw) if !raw.is_empty() => Some(CompiledBody::Text(
                    TemplateString::parse(raw).map_err(ConfigError::InvalidScenario)?,
                )),
                _ => None,
            },
            BodyMode::File => {
                let path = step
                    .body_file
                    .as_ref()
                    .ok_or(ConfigError::EmptyBodyFilePath)?;
                let path_str = path.to_string_lossy();
                let trimmed = path_str.trim();
                let bytes = fs::read(trimmed).map_err(|source| ConfigError::BodyFileReadError {
                    path: trimmed.to_string(),
                    source,
                })?;
                if bytes.is_empty() {
                    None
                } else if let Ok(text) = std::str::from_utf8(&bytes) {
                    Some(CompiledBody::Text(
                        TemplateString::parse(text).map_err(ConfigError::InvalidScenario)?,
                    ))
                } else {
                    Some(CompiledBody::Binary(bytes))
                }
            }
        };

        let timeout = match step.timeout.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => Some(parse_timeout(raw).map_err(ConfigError::InvalidTimeoutFormat)?),
            None => None,
        };

        let think_time = match step
            .think_time
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => parse_duration(raw).map_err(ConfigError::InvalidScenario)?,
            None => Duration::ZERO,
        };

        let mut compiled_extractors = Vec::with_capacity(step.extractors.len());
        for ex in &step.extractors {
            let name = ex.name.trim().to_string();
            let sensitive = ex.sensitive || is_sensitive_var_name(&name);
            if sensitive {
                sensitive_var_names.insert(name.clone());
            }
            let source = match &ex.source {
                ExtractorSource::JsonPath { path } => CompiledExtractorSource::JsonPath(
                    parse_json_path(path).map_err(ConfigError::InvalidScenario)?,
                ),
                ExtractorSource::Header { name: h_name } => CompiledExtractorSource::Header(
                    HeaderName::from_bytes(h_name.trim().as_bytes())
                        .map_err(|err| ConfigError::InvalidHeader(err.to_string()))?,
                ),
                ExtractorSource::Cookie { name: c_name } => {
                    CompiledExtractorSource::Cookie(c_name.trim().to_string())
                }
                ExtractorSource::StatusCode => CompiledExtractorSource::StatusCode,
            };

            compiled_extractors.push(CompiledExtractor {
                name,
                source,
                source_label: ex.source.display_label(),
                required: ex.required,
                sensitive,
            });
        }

        let mut compiled_checks = Vec::with_capacity(step.assertions.json_checks.len());
        for check in &step.assertions.json_checks {
            compiled_checks.push(CompiledJsonCheck {
                path: parse_json_path(&check.path).map_err(ConfigError::InvalidScenario)?,
                op: check.op,
                expected: check.expected.clone(),
            });
        }

        compiled_steps.push(CompiledStep {
            step_index: compiled_steps.len(),
            order_number: raw_idx + 1,
            id: step.id.trim().to_string(),
            name: step.name.trim().to_string(),
            method: Method::from(step.method),
            url_template,
            target_preview: sanitize_url_string_preserving_templates(&combined_url),
            headers: compiled_headers,
            body: compiled_body,
            timeout,
            think_time,
            extractors: compiled_extractors,
            status_expectation: step.assertions.status.clone(),
            json_checks: compiled_checks,
            on_failure: step.on_failure,
        });
    }

    Ok(CompiledScenario {
        name: input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        base_url: normalized_base,
        initial_variables,
        steps: compiled_steps,
        unique_host_count: unique_origins.len().max(1),
        sensitive_var_names,
    })
}

/// Formats a [`ScenarioReviewReport`] as a terminal-friendly `comfy-table` sequence matrix.
pub fn render_review_report(review: &ScenarioReviewReport) -> String {
    let mut out = String::new();
    let title = review.name.as_deref().unwrap_or("Ordered API Sequence");
    out.push_str(&format!("=== Sequence Review: {title} ===\n"));
    if let Some(ref base) = review.base_url {
        out.push_str(&format!("Base URL: {base}\n"));
    }
    out.push_str(&format!(
        "Steps: {}/{} enabled | Unique hosts: {} | Status: {}\n\n",
        review.enabled_step_count,
        review.total_step_count,
        review.unique_hosts,
        if review.valid { "VALID" } else { "INVALID" }
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
            Cell::new("Consumes"),
            Cell::new("Produces"),
            Cell::new("Expects"),
            Cell::new("On Failure"),
        ]);

    for step in &review.steps {
        let state_suffix = if step.enabled { "" } else { " [disabled]" };
        let on_failure = match step.on_failure {
            StepFailurePolicy::AbortIteration => "abort",
            StepFailurePolicy::Continue => "continue",
        };
        table.add_row(vec![
            Cell::new(step.order_number.to_string()),
            Cell::new(format!("{}{}", step.name, state_suffix)),
            Cell::new(&step.method),
            Cell::new(&step.target_preview),
            Cell::new(if step.consumes.is_empty() {
                "-".to_string()
            } else {
                step.consumes.join(", ")
            }),
            Cell::new(if step.produces.is_empty() {
                "-".to_string()
            } else {
                step.produces.join(", ")
            }),
            Cell::new(&step.assertions_summary),
            Cell::new(on_failure),
        ]);
    }
    out.push_str(&table.to_string());
    out.push('\n');

    if !review.warnings.is_empty() {
        out.push_str("\nWarnings:\n");
        for warn in &review.warnings {
            out.push_str(&format!("  - {warn}\n"));
        }
    }

    if !review.errors.is_empty() {
        out.push_str("\nErrors:\n");
        for err in &review.errors {
            out.push_str(&format!("  - {err}\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::HttpMethod;
    use crate::scenario::config::StepExtractorInput;

    fn base_step(id: &str, name: &str, url: &str) -> ApiStepInput {
        ApiStepInput {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            method: HttpMethod::Get,
            url: url.to_string(),
            headers: Vec::new(),
            body_mode: BodyMode::None,
            body: None,
            body_file: None,
            timeout: None,
            think_time: None,
            extractors: Vec::new(),
            assertions: StepAssertionsInput::default(),
            on_failure: StepFailurePolicy::AbortIteration,
        }
    }

    use crate::scenario::config::StepAssertionsInput;

    #[test]
    fn test_validate_scenario_catches_out_of_order_and_disabled_variable_refs() {
        let mut s1 = base_step("s1", "Use Token Early", "/orders/{{token}}");
        s1.enabled = true;
        let mut s2 = base_step("s2", "Login", "/auth");
        s2.extractors.push(StepExtractorInput {
            name: "token".to_string(),
            source: ExtractorSource::JsonPath {
                path: "$.token".to_string(),
            },
            required: true,
            sensitive: true,
        });

        let scenario = ScenarioConfigInput {
            base_url: Some("https://api.example.com".to_string()),
            steps: vec![s1, s2],
            ..Default::default()
        };

        let (report, review) = validate_scenario(&scenario, Duration::from_secs(5), 10, None);
        assert!(!report.valid);
        assert!(!review.valid);
        assert!(report
            .field_errors
            .get("scenario.steps.0.url")
            .unwrap()
            .contains("before it is extracted by step #2"));
    }

    #[test]
    fn test_validate_scenario_valid_chain_and_unused_warning() {
        let mut s1 = base_step("s1", "Login", "/auth");
        s1.method = HttpMethod::Post;
        s1.extractors.push(StepExtractorInput {
            name: "auth_token".to_string(),
            source: ExtractorSource::JsonPath {
                path: "$.token".to_string(),
            },
            required: true,
            sensitive: true,
        });
        s1.extractors.push(StepExtractorInput {
            name: "unused_field".to_string(),
            source: ExtractorSource::StatusCode,
            required: false,
            sensitive: false,
        });

        let mut s2 = base_step("s2", "Profile", "/me?vu={{__vu_id}}");
        s2.headers.push(HeaderEntry {
            name: "Authorization".to_string(),
            value: "Bearer {{auth_token}}".to_string(),
            enabled: true,
        });

        let scenario = ScenarioConfigInput {
            name: Some("Auth Flow".to_string()),
            base_url: Some("https://api.example.com".to_string()),
            steps: vec![s1, s2],
            ..Default::default()
        };

        let (report, review) = validate_scenario(&scenario, Duration::from_secs(5), 50, None);
        assert!(report.valid, "errors: {:?}", report.field_errors);
        assert_eq!(review.enabled_step_count, 2);
        assert_eq!(review.steps[0].produces, vec!["auth_token", "unused_field"]);
        assert_eq!(review.steps[1].consumes, vec!["__vu_id", "auth_token"]);
        assert!(review.warnings.iter().any(|w| w.contains("unused_field")));

        let compiled = compile_scenario(&scenario, Duration::from_secs(5)).expect("compile");
        assert_eq!(compiled.steps.len(), 2);
        assert_eq!(compiled.unique_host_count, 1);
    }
}
