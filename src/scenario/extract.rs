//! Response body JSONPath, header, cookie, and status extractors.

use reqwest::header::{HeaderMap, SET_COOKIE};
use serde_json::Value;

/// Maximum response body size buffered in memory when JSON extraction or assertions are enabled (1 MiB).
pub const MAX_EXTRACTION_BODY_BYTES: usize = 1024 * 1024;

/// Single navigation step within a [`JsonPathExpr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// Compiled lightweight dot/bracket JSON path expression (e.g. `$.data.items[0].id`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPathExpr {
    pub raw: String,
    segments: Vec<PathSegment>,
}

impl JsonPathExpr {
    /// Parses a JSON path expression supporting `$.a.b`, `a.b`, `$.items[0].id`, and `$.meta["x-id"]`.
    pub fn parse(raw: &str) -> Result<Self, String> {
        parse_json_path(raw)
    }

    /// Resolves this path against a parsed [`serde_json::Value`].
    pub fn resolve<'a>(&self, root: &'a Value) -> Option<&'a Value> {
        let mut cur = root;
        for seg in &self.segments {
            match seg {
                PathSegment::Key(key) => {
                    cur = cur.as_object()?.get(key)?;
                }
                PathSegment::Index(idx) => {
                    cur = cur.as_array()?.get(*idx)?;
                }
            }
        }
        Some(cur)
    }
}

/// Parses a dot/bracket JSON path into a compiled [`JsonPathExpr`].
pub fn parse_json_path(raw: &str) -> Result<JsonPathExpr, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("JSON path cannot be empty".to_string());
    }

    let mut rest = trimmed;
    if let Some(after_dollar) = rest.strip_prefix('$') {
        rest = after_dollar;
        if rest.is_empty() {
            return Ok(JsonPathExpr {
                raw: trimmed.to_string(),
                segments: Vec::new(),
            });
        }
        if let Some(after_dot) = rest.strip_prefix('.') {
            rest = after_dot;
            if rest.is_empty() {
                return Err(format!("trailing '.' in JSON path '{trimmed}'"));
            }
        } else if !rest.starts_with('[') {
            return Err(format!(
                "invalid JSON path '{trimmed}': expected '.' or '[' after '$'"
            ));
        }
    }

    let mut segments = Vec::new();
    let bytes = rest.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'.' => {
                if i == 0 || i + 1 == bytes.len() {
                    return Err(format!("unexpected '.' in JSON path '{trimmed}'"));
                }
                i += 1;
            }
            b'[' => {
                i += 1;
                if i >= bytes.len() {
                    return Err(format!("unclosed '[' in JSON path '{trimmed}'"));
                }
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    let quote = bytes[i];
                    i += 1;
                    let start = i;
                    while i < bytes.len() && bytes[i] != quote {
                        i += 1;
                    }
                    if i >= bytes.len() {
                        return Err(format!("unterminated quoted key in JSON path '{trimmed}'"));
                    }
                    let key = &rest[start..i];
                    if key.is_empty() {
                        return Err(format!("empty quoted key in JSON path '{trimmed}'"));
                    }
                    i += 1;
                    if i >= bytes.len() || bytes[i] != b']' {
                        return Err(format!(
                            "expected ']' after quoted key in JSON path '{trimmed}'"
                        ));
                    }
                    i += 1;
                    segments.push(PathSegment::Key(key.to_string()));
                } else {
                    let start = i;
                    while i < bytes.len() && bytes[i] != b']' {
                        i += 1;
                    }
                    if i >= bytes.len() {
                        return Err(format!("unclosed '[' in JSON path '{trimmed}'"));
                    }
                    let idx_str = rest[start..i].trim();
                    let idx = idx_str.parse::<usize>().map_err(|_| {
                        format!(
                            "invalid array index '{idx_str}' in JSON path '{trimmed}': expected non-negative integer or quoted key"
                        )
                    })?;
                    i += 1;
                    segments.push(PathSegment::Index(idx));
                }
                if i < bytes.len() && bytes[i] == b'.' {
                    i += 1;
                    if i >= bytes.len() {
                        return Err(format!("trailing '.' in JSON path '{trimmed}'"));
                    }
                }
            }
            _ => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'.' && bytes[i] != b'[' {
                    i += 1;
                }
                let key = rest[start..i].trim();
                if key.is_empty() {
                    return Err(format!("empty field name in JSON path '{trimmed}'"));
                }
                segments.push(PathSegment::Key(key.to_string()));
                if i < bytes.len() && bytes[i] == b'.' {
                    i += 1;
                    if i >= bytes.len() {
                        return Err(format!("trailing '.' in JSON path '{trimmed}'"));
                    }
                }
            }
        }
    }

    Ok(JsonPathExpr {
        raw: trimmed.to_string(),
        segments,
    })
}

/// Converts a scalar JSON [`Value`] into a template-substitutable string.
pub fn json_value_to_string(value: &Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Null => Err("extracted JSON value is null".to_string()),
        Value::Array(_) => Err("extracted JSON value is an array, expected scalar".to_string()),
        Value::Object(_) => Err("extracted JSON value is an object, expected scalar".to_string()),
    }
}

/// Extracts the value of cookie `cookie_name` from HTTP `Set-Cookie` response headers.
pub fn extract_cookie_from_headers(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let target = cookie_name.trim();
    if target.is_empty() {
        return None;
    }

    for header_val in headers.get_all(SET_COOKIE) {
        let Ok(raw) = header_val.to_str() else {
            continue;
        };
        let first_pair = raw.split(';').next().unwrap_or("").trim();
        if let Some((name, val)) = first_pair.split_once('=') {
            if name.trim() == target {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;
    use serde_json::json;

    #[test]
    fn test_json_path_resolution() {
        let doc = json!({
            "data": {
                "access_token": "tok_123",
                "items": [
                    { "id": 101, "active": true },
                    { "id": 202, "active": false }
                ],
                "meta": {
                    "x-request-id": "req-abc"
                }
            }
        });

        let p1 = parse_json_path("$.data.access_token").expect("parse");
        assert_eq!(
            json_value_to_string(p1.resolve(&doc).expect("found")).expect("scalar"),
            "tok_123"
        );

        let p2 = parse_json_path("data.items[1].id").expect("parse");
        assert_eq!(
            json_value_to_string(p2.resolve(&doc).expect("found")).expect("scalar"),
            "202"
        );

        let p3 = parse_json_path("$.data.meta[\"x-request-id\"]").expect("parse");
        assert_eq!(
            json_value_to_string(p3.resolve(&doc).expect("found")).expect("scalar"),
            "req-abc"
        );

        let p4 = parse_json_path("$.data.items[0].active").expect("parse");
        assert_eq!(
            json_value_to_string(p4.resolve(&doc).expect("found")).expect("scalar"),
            "true"
        );

        assert!(parse_json_path("$.data").is_ok());
        assert!(json_value_to_string(parse_json_path("$.data").unwrap().resolve(&doc).unwrap()).is_err());
        assert!(parse_json_path("").is_err());
        assert!(parse_json_path("$.items[abc]").is_err());
        assert!(parse_json_path("$.data.").is_err());
    }

    #[test]
    fn test_extract_cookie_from_headers() {
        let mut headers = HeaderMap::new();
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("theme=dark; Path=/; SameSite=Lax"),
        );
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("session_id=abc987; Path=/; HttpOnly; Secure"),
        );

        assert_eq!(
            extract_cookie_from_headers(&headers, "session_id").as_deref(),
            Some("abc987")
        );
        assert_eq!(
            extract_cookie_from_headers(&headers, "theme").as_deref(),
            Some("dark")
        );
        assert_eq!(extract_cookie_from_headers(&headers, "missing"), None);
    }
}
