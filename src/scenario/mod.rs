//! Ordered multi-step API sequence configuration, static review, dry-run, and runtime execution support.

pub mod config;
pub mod dry_run;
pub mod extract;
pub mod template;
pub mod validate;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::BenchmarkConfigInput;

pub use config::{
    is_sensitive_var_name, redact_scenario_input, sanitize_url_string_preserving_templates,
    scrub_sensitive_text, ApiStepInput, AssertionOutcome, CompiledBody, CompiledExtractor,
    CompiledExtractorSource, CompiledHeader, CompiledJsonCheck, CompiledScenario, CompiledStep,
    ExtractedVarOutcome, ExtractorSource, JsonCheckInput, JsonCheckOp, RequestMode,
    ScenarioConfigInput, ScenarioMetaSummary, ScenarioStepMeta, ScenarioVariableInput,
    StatusExpectation, StepAssertionsInput, StepEvaluationResult, StepExtractorInput,
    StepFailurePolicy,
};
pub use dry_run::{
    execute_dry_run, render_dry_run_report, DryRunAssertionResult, DryRunExtractedVar,
    DryRunStepTrace, DryRunTraceReport, DRY_RUN_OVERALL_TIMEOUT, DRY_RUN_PREVIEW_LIMIT,
};
pub use extract::{
    extract_cookie_from_headers, json_value_to_string, parse_json_path, JsonPathExpr,
    MAX_EXTRACTION_BODY_BYTES,
};
pub use template::{
    is_builtin_var, is_valid_var_name, parse_template, BuiltInVars, TemplatePart, TemplateString,
    BUILTIN_VARS,
};
pub use validate::{
    compile_scenario, render_review_report, validate_scenario, ScenarioReviewReport,
    StepReviewItem,
};

/// Loads an ordered multi-step scenario JSON file into a [`BenchmarkConfigInput`].
///
/// Accepts either a full [`BenchmarkConfigInput`] envelope (with `scenario`) or a bare
/// [`ScenarioConfigInput`] document (`{ base_url, steps, ... }`), normalizing `mode` to
/// [`RequestMode::Sequence`].
pub fn load_scenario_file(path: &Path) -> Result<BenchmarkConfigInput> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read scenario file '{}'", path.display()))?;
    parse_scenario_json(&raw)
        .with_context(|| format!("failed to parse scenario JSON from '{}'", path.display()))
}

/// Parses raw JSON text as either a [`BenchmarkConfigInput`] envelope or a bare [`ScenarioConfigInput`].
pub fn parse_scenario_json(raw: &str) -> Result<BenchmarkConfigInput> {
    let value: serde_json::Value =
        serde_json::from_str(raw).context("invalid JSON syntax in scenario document")?;

    if value.get("scenario").is_some() {
        let mut cfg: BenchmarkConfigInput = serde_json::from_value(value)
            .context("invalid scenario benchmark envelope")?;
        cfg.mode = RequestMode::Sequence;
        return Ok(cfg);
    }

    if value.get("steps").is_some() {
        let scenario: ScenarioConfigInput =
            serde_json::from_value(value).context("invalid scenario configuration object")?;
        return Ok(BenchmarkConfigInput {
            mode: RequestMode::Sequence,
            scenario: Some(scenario),
            ..Default::default()
        });
    }

    let mut cfg: BenchmarkConfigInput =
        serde_json::from_value(value).context("expected 'scenario' or 'steps' field in scenario JSON")?;
    if cfg.scenario.is_some() {
        cfg.mode = RequestMode::Sequence;
    }
    Ok(cfg)
}
