use std::process::Command;

use rusty_loader::changelog::{
    about_payload, changelog_payload, full_changelog, latest_release_notes, parse_changelog_str,
    parsed_changelog, APP_NAME, APP_VERSION,
};

#[test]
fn test_embedded_changelog_and_latest_release_notes() {
    let raw = full_changelog();
    assert!(raw.contains("# Changelog"));
    assert!(raw.contains(&format!("## [{APP_VERSION}]")));

    let releases = parsed_changelog();
    assert!(
        !releases.is_empty(),
        "embedded CHANGELOG.md must contain at least one release entry"
    );

    let latest = latest_release_notes().expect("latest_release_notes should return Some");
    assert_eq!(latest.version, APP_VERSION);
    assert_eq!(latest.date.as_deref(), Some("2026-11-21"));
    assert!(latest.notes.contains("### Features"));

    let about = about_payload();
    assert_eq!(about.name, APP_NAME);
    assert_eq!(about.version, APP_VERSION);
    assert_eq!(about.latest_change, Some(latest.clone()));

    let payload = changelog_payload();
    assert_eq!(payload.current_version, APP_VERSION);
    assert_eq!(payload.raw_markdown, raw);
    assert_eq!(payload.releases, releases);
}

#[test]
fn test_parse_changelog_str_multiple_formats_and_unreleased() {
    let sample = r#"# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Features
- Work in progress feature that should not appear as a versioned release

## [0.3.0](https://github.com/seanr89/rusty-loader/compare/v0.2.1...v0.3.0) (2026-12-01)

### Features
- Added distributed worker coordination
- Added live HDR percentile streaming

### Bug Fixes
- Fixed connection drain timeout edge case

## [v0.2.1] - 2026-11-25

### Bug Fixes
- Resolved header redaction case sensitivity

## 0.2.0 - 2026-11-22

### Features
- Initial GUI dashboard release
"#;

    let entries = parse_changelog_str(sample);
    assert_eq!(entries.len(), 3);

    assert_eq!(entries[0].version, "0.3.0");
    assert_eq!(entries[0].date.as_deref(), Some("2026-12-01"));
    assert!(entries[0].notes.contains("distributed worker coordination"));
    assert!(entries[0].notes.contains("### Bug Fixes"));

    assert_eq!(entries[1].version, "0.2.1");
    assert_eq!(entries[1].date.as_deref(), Some("2026-11-25"));
    assert!(entries[1]
        .notes
        .contains("Resolved header redaction case sensitivity"));

    assert_eq!(entries[2].version, "0.2.0");
    assert_eq!(entries[2].date.as_deref(), Some("2026-11-22"));
    assert!(entries[2].notes.contains("Initial GUI dashboard release"));
}

#[test]
fn test_cli_binary_changelog_flag_outputs_embedded_changelog() {
    let bin = env!("CARGO_BIN_EXE_rusty-loader");
    let output = Command::new(bin)
        .arg("--changelog")
        .output()
        .expect("should execute rusty-loader --changelog");

    assert!(
        output.status.success(),
        "rusty-loader --changelog should exit 0 (stderr: {})",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Changelog"));
    assert!(stdout.contains(&format!("## [{APP_VERSION}]")));
    assert_eq!(stdout, full_changelog());
}
