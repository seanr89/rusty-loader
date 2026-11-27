//! Pre-compiled `{{variable}}` string template parser and per-VU renderer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Built-in runtime variables automatically available in every VU iteration.
pub const BUILTIN_VARS: [&str; 3] = ["__vu_id", "__iteration", "__timestamp_ms"];

/// Runtime values for built-in per-VU iteration variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltInVars {
    pub vu_id: usize,
    pub iteration: u64,
    pub timestamp_ms: u64,
}

impl BuiltInVars {
    /// Looks up a built-in variable name, returning `None` for non-built-ins.
    pub fn get(&self, name: &str) -> Option<String> {
        match name {
            "__vu_id" => Some(self.vu_id.to_string()),
            "__iteration" => Some(self.iteration.to_string()),
            "__timestamp_ms" => Some(self.timestamp_ms.to_string()),
            _ => None,
        }
    }
}

/// Returns `true` if `name` is one of the reserved built-in variable names.
pub fn is_builtin_var(name: &str) -> bool {
    BUILTIN_VARS.contains(&name)
}

/// Returns `true` if `name` is a valid variable identifier (`[A-Za-z_][A-Za-z0-9_]*`).
pub fn is_valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Single segment of a pre-compiled [`TemplateString`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePart {
    Literal(Arc<str>),
    Var(String),
}

/// Pre-compiled string template supporting `{{variable}}` interpolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateString {
    parts: Vec<TemplatePart>,
    referenced_vars: Vec<String>,
}

impl TemplateString {
    /// Compiles a literal string containing no `{{...}}` placeholders.
    pub fn literal(raw: impl Into<Arc<str>>) -> Self {
        let arc: Arc<str> = raw.into();
        let parts = if arc.is_empty() {
            Vec::new()
        } else {
            vec![TemplatePart::Literal(arc)]
        };
        Self {
            parts,
            referenced_vars: Vec::new(),
        }
    }

    /// Parses and compiles a template string with `{{var_name}}` placeholders.
    pub fn parse(raw: &str) -> Result<Self, String> {
        parse_template(raw)
    }

    /// Returns `true` when this template contains no variable placeholders.
    pub fn is_static(&self) -> bool {
        self.referenced_vars.is_empty()
    }

    /// Returns the static string slice if this template has no variable placeholders.
    pub fn static_str(&self) -> Option<&str> {
        if !self.is_static() {
            return None;
        }
        match self.parts.as_slice() {
            [] => Some(""),
            [TemplatePart::Literal(s)] => Some(s.as_ref()),
            _ => None,
        }
    }

    /// Returns the deduplicated list of variable names referenced by this template in appearance order.
    pub fn referenced_vars(&self) -> &[String] {
        &self.referenced_vars
    }

    /// Renders this template using `vars` and `built_ins`, replacing unknown variables with `""`.
    pub fn render(&self, vars: &HashMap<String, String>, built_ins: &BuiltInVars) -> String {
        if let Some(static_val) = self.static_str() {
            return static_val.to_string();
        }

        let mut out = String::with_capacity(self.parts.len() * 16);
        for part in &self.parts {
            match part {
                TemplatePart::Literal(lit) => out.push_str(lit),
                TemplatePart::Var(name) => {
                    if let Some(val) = built_ins.get(name) {
                        out.push_str(&val);
                    } else if let Some(val) = vars.get(name) {
                        out.push_str(val);
                    }
                }
            }
        }
        out
    }

    /// Renders this template replacing every `{{var}}` placeholder with a safe URL/header token.
    pub fn render_with_dummy(&self) -> String {
        if let Some(static_val) = self.static_str() {
            return static_val.to_string();
        }
        let mut out = String::with_capacity(self.parts.len() * 16);
        for part in &self.parts {
            match part {
                TemplatePart::Literal(lit) => out.push_str(lit),
                TemplatePart::Var(_) => out.push_str("rlvar"),
            }
        }
        out
    }
}

/// Parses `raw` into a compiled [`TemplateString`], rejecting malformed `{{...}}` tokens.
pub fn parse_template(raw: &str) -> Result<TemplateString, String> {
    if !raw.contains("{{") {
        return Ok(TemplateString::literal(raw));
    }

    let mut parts = Vec::new();
    let mut referenced_vars = Vec::new();
    let mut seen_vars = HashSet::new();
    let mut rest = raw;

    while let Some(open_idx) = rest.find("{{") {
        if open_idx > 0 {
            parts.push(TemplatePart::Literal(Arc::from(&rest[..open_idx])));
        }

        let after_open = &rest[open_idx + 2..];
        let Some(close_idx) = after_open.find("}}") else {
            return Err("unclosed template placeholder '{{' (missing matching '}}')".to_string());
        };

        let inner = after_open[..close_idx].trim();
        if inner.is_empty() {
            return Err("empty template placeholder '{{}}' is not allowed".to_string());
        }
        if inner.contains("{{") {
            return Err(format!("nested template placeholder in '{{{{{inner}}}}}' is not supported"));
        }
        if !is_valid_var_name(inner) {
            return Err(format!(
                "invalid variable name '{inner}': expected [A-Za-z_][A-Za-z0-9_]*"
            ));
        }

        let var_name = inner.to_string();
        if seen_vars.insert(var_name.clone()) {
            referenced_vars.push(var_name.clone());
        }
        parts.push(TemplatePart::Var(var_name));

        rest = &after_open[close_idx + 2..];
    }

    if !rest.is_empty() {
        parts.push(TemplatePart::Literal(Arc::from(rest)));
    }

    Ok(TemplateString {
        parts,
        referenced_vars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_template_fast_path() {
        let tpl = parse_template("https://api.example.com/v1/ping").expect("parse static");
        assert!(tpl.is_static());
        assert_eq!(tpl.static_str(), Some("https://api.example.com/v1/ping"));
        assert!(tpl.referenced_vars().is_empty());
    }

    #[test]
    fn test_template_parse_and_render() {
        let tpl = parse_template("/users/{{ user_id }}/orders/{{__vu_id}}?iter={{__iteration}}")
            .expect("parse template");
        assert!(!tpl.is_static());
        assert_eq!(
            tpl.referenced_vars(),
            &[
                "user_id".to_string(),
                "__vu_id".to_string(),
                "__iteration".to_string()
            ]
        );

        let mut vars = HashMap::new();
        vars.insert("user_id".to_string(), "u-42".to_string());
        let built_ins = BuiltInVars {
            vu_id: 7,
            iteration: 3,
            timestamp_ms: 1_700_000_000_000,
        };
        assert_eq!(
            tpl.render(&vars, &built_ins),
            "/users/u-42/orders/7?iter=3"
        );
    }

    #[test]
    fn test_template_rejects_malformed_placeholders() {
        assert!(parse_template("Bearer {{token").is_err());
        assert!(parse_template("Bearer {{}}").is_err());
        assert!(parse_template("Bearer {{1bad}}").is_err());
        assert!(parse_template("Bearer {{bad-name}}").is_err());
    }
}
