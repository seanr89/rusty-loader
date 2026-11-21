//! Embedded application version and `CHANGELOG.md` release notes parser.

use serde::{Deserialize, Serialize};

/// Application package name from `Cargo.toml`.
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// Current semantic version from `Cargo.toml`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Embedded `CHANGELOG.md` provisioned at compile time for offline access.
pub const EMBEDDED_CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// A single parsed versioned release entry from `CHANGELOG.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseEntry {
    /// Semantic version string (e.g. `"0.1.0"`).
    pub version: String,
    /// Optional ISO release date (e.g. `"2026-11-21"`).
    pub date: Option<String>,
    /// Markdown body containing categorized release notes for this version.
    pub notes: String,
}

/// Complete structured changelog payload returned by `GET /api/changelog`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangelogPayload {
    /// Current binary version (`CARGO_PKG_VERSION`).
    pub current_version: String,
    /// Raw embedded `CHANGELOG.md` markdown text.
    pub raw_markdown: String,
    /// Parsed chronological list of release entries (newest first).
    pub releases: Vec<ReleaseEntry>,
}

/// Application metadata and latest release notes returned by `GET /api/about`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AboutPayload {
    /// Application name (`rusty-loader`).
    pub name: String,
    /// Current compiled version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// Short description of the application.
    pub description: String,
    /// Most recent versioned release section parsed from `CHANGELOG.md`.
    pub latest_change: Option<ReleaseEntry>,
}

/// Returns the full raw embedded `CHANGELOG.md` text.
pub fn full_changelog() -> &'static str {
    EMBEDDED_CHANGELOG
}

/// Parses the embedded `CHANGELOG.md` into a list of [`ReleaseEntry`] items (newest first).
pub fn parsed_changelog() -> Vec<ReleaseEntry> {
    parse_changelog_str(EMBEDDED_CHANGELOG)
}

/// Returns the newest release entry from the embedded `CHANGELOG.md`.
pub fn latest_release_notes() -> Option<ReleaseEntry> {
    parsed_changelog().into_iter().next()
}

/// Constructs the [`AboutPayload`] for the current binary.
pub fn about_payload() -> AboutPayload {
    let releases = parsed_changelog();
    let latest_change = releases
        .iter()
        .find(|entry| entry.version == APP_VERSION)
        .cloned()
        .or_else(|| releases.into_iter().next());

    AboutPayload {
        name: APP_NAME.to_string(),
        version: APP_VERSION.to_string(),
        description: env!("CARGO_PKG_DESCRIPTION").to_string(),
        latest_change,
    }
}

/// Constructs the [`ChangelogPayload`] for the current binary.
pub fn changelog_payload() -> ChangelogPayload {
    ChangelogPayload {
        current_version: APP_VERSION.to_string(),
        raw_markdown: EMBEDDED_CHANGELOG.to_string(),
        releases: parsed_changelog(),
    }
}

/// Parses a Keep-a-Changelog / release-please / git-cliff formatted markdown string
/// into structured [`ReleaseEntry`] items.
///
/// Supported heading formats include:
/// - `## [0.2.0] - 2026-11-21`
/// - `## [0.2.0](https://...) (2026-11-21)`
/// - `## v0.2.0 - 2026-11-21`
/// - `## 0.2.0 (2026-11-21)`
pub fn parse_changelog_str(markdown: &str) -> Vec<ReleaseEntry> {
    let mut entries = Vec::new();
    let mut current_header: Option<(String, Option<String>)> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in markdown.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some((version, date)) = current_header.take() {
                entries.push(ReleaseEntry {
                    version,
                    date,
                    notes: current_lines.join("\n").trim().to_string(),
                });
                current_lines.clear();
            }

            if let Some((version, date)) = parse_release_heading(rest.trim()) {
                if !version.eq_ignore_ascii_case("unreleased") {
                    current_header = Some((version, date));
                }
            }
        } else if current_header.is_some() {
            current_lines.push(line);
        }
    }

    if let Some((version, date)) = current_header.take() {
        entries.push(ReleaseEntry {
            version,
            date,
            notes: current_lines.join("\n").trim().to_string(),
        });
    }

    entries
}

fn parse_release_heading(heading: &str) -> Option<(String, Option<String>)> {
    if heading.is_empty() {
        return None;
    }

    // Handle bracketed version headings: `[0.1.0] - 2026-11-21` or `[0.1.0](url) (2026-11-21)`
    let (raw_version, remainder) = if let Some(after_open) = heading.strip_prefix('[') {
        let close_idx = after_open.find(']')?;
        let ver = &after_open[..close_idx];
        let mut rem = after_open[close_idx + 1..].trim();
        // Skip optional markdown link target `(https://...)` immediately following `[version]`
        if rem.starts_with('(') {
            if let Some(end_paren) = rem.find(')') {
                let inside = &rem[1..end_paren];
                if inside.contains("://") || inside.starts_with('/') || inside.starts_with('#') {
                    rem = rem[end_paren + 1..].trim();
                }
            }
        }
        (ver, rem)
    } else {
        let mut parts = heading.splitn(2, char::is_whitespace);
        let ver = parts.next()?;
        let rem = parts.next().unwrap_or("").trim();
        (ver, rem)
    };

    let clean_version = raw_version
        .trim()
        .trim_start_matches(['v', 'V'])
        .trim()
        .to_string();

    if clean_version.is_empty() {
        return None;
    }

    let date = extract_date(remainder);
    Some((clean_version, date))
}

fn extract_date(remainder: &str) -> Option<String> {
    let trimmed = remainder
        .trim()
        .trim_start_matches(['-', '–', '—', ':'])
        .trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = trimmed
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();

    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}
