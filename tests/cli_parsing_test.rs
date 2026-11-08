use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use reqwest::Method;
use rusty_loader::cli::{parse_duration, parse_header, parse_timeout, Cli, HttpMethod};
use rusty_loader::config::{BenchmarkConfig, ConfigError};
use tempfile::NamedTempFile;

#[test]
fn test_default_values() {
    let cli = Cli::try_parse_from(["rusty-loader", "--url", "https://api.example.com/v1/ping"])
        .expect("CLI should parse with only --url");

    assert_eq!(cli.url, "https://api.example.com/v1/ping");
    assert_eq!(cli.concurrency, 700);
    assert_eq!(cli.duration, Duration::from_secs(30));
    assert_eq!(cli.method, HttpMethod::Get);
    assert!(cli.headers.is_empty());
    assert!(cli.body.is_none());
    assert_eq!(cli.timeout, Duration::from_millis(5000));
    assert_eq!(cli.ramp_up, Duration::ZERO);
    assert!(cli.output_json.is_none());

    let config = BenchmarkConfig::from_cli(cli).expect("Default CLI should produce valid config");
    assert_eq!(config.url.as_str(), "https://api.example.com/v1/ping");
    assert_eq!(config.concurrency, 700);
    assert_eq!(config.duration, Duration::from_secs(30));
    assert_eq!(config.method, Method::GET);
    assert!(config.headers.is_empty());
    assert!(config.body.is_none());
    assert_eq!(config.timeout, Duration::from_millis(5000));
    assert_eq!(config.ramp_up, Duration::ZERO);
    assert!(config.output_json.is_none());
}

#[test]
fn test_duration_formats() {
    assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
    assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
    assert_eq!(parse_duration("1000ms").unwrap(), Duration::from_secs(1));
    assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
    assert_eq!(parse_duration("0s").unwrap(), Duration::ZERO);

    assert!(parse_duration("").is_err());
    assert!(parse_duration("abc").is_err());
    assert!(parse_duration("-10s").is_err());
    assert!(parse_duration("ms").is_err());
}

#[test]
fn test_timeout_formats() {
    // Raw integer for --timeout is interpreted as milliseconds
    assert_eq!(parse_timeout("2000").unwrap(), Duration::from_millis(2000));
    assert_eq!(
        parse_timeout("5000ms").unwrap(),
        Duration::from_millis(5000)
    );
    assert_eq!(parse_timeout("2s").unwrap(), Duration::from_secs(2));
    assert_eq!(parse_timeout("1m").unwrap(), Duration::from_secs(60));

    let cli = Cli::try_parse_from([
        "api-load-tester",
        "--url",
        "https://api.example.com/v1/ping",
        "--concurrency",
        "700",
        "--duration",
        "60s",
        "--timeout",
        "2000",
    ])
    .expect("CLI should parse raw integer timeout in ms");

    assert_eq!(cli.timeout, Duration::from_millis(2000));
    assert_eq!(cli.duration, Duration::from_secs(60));
}

#[test]
fn test_http_methods() {
    for (flag, expected_method) in [
        ("GET", Method::GET),
        ("POST", Method::POST),
        ("PUT", Method::PUT),
        ("DELETE", Method::DELETE),
        ("PATCH", Method::PATCH),
        ("HEAD", Method::HEAD),
        ("post", Method::POST),
    ] {
        let cli = Cli::try_parse_from([
            "rusty-loader",
            "-u",
            "https://api.example.com/v1/resource",
            "-m",
            flag,
        ])
        .unwrap_or_else(|e| panic!("failed to parse method {flag}: {e}"));

        let config = BenchmarkConfig::from_cli(cli).unwrap();
        assert_eq!(config.method, expected_method);
    }

    let invalid = Cli::try_parse_from([
        "rusty-loader",
        "-u",
        "https://api.example.com/v1/resource",
        "-m",
        "INVALID_METHOD",
    ]);
    assert!(invalid.is_err());
}

#[test]
fn test_repeatable_headers_parsing() {
    let cli = Cli::try_parse_from([
        "rusty-loader",
        "-u",
        "https://api.example.com/v1/query",
        "-H",
        "Authorization: Bearer token123",
        "-H",
        "Content-Type: application/json",
        "-H",
        "X-Custom-Header: value:with:colons",
    ])
    .expect("CLI should parse multiple -H flags");

    assert_eq!(cli.headers.len(), 3);

    let config = BenchmarkConfig::from_cli(cli).expect("Headers should validate cleanly");
    assert_eq!(
        config
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer token123"
    );
    assert_eq!(
        config
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    assert_eq!(
        config
            .headers
            .get("x-custom-header")
            .unwrap()
            .to_str()
            .unwrap(),
        "value:with:colons"
    );
}

#[test]
fn test_malformed_headers_rejected() {
    assert!(parse_header("MissingColonHeader").is_err());
    assert!(parse_header(": value_without_key").is_err());

    let cli = Cli::try_parse_from([
        "rusty-loader",
        "-u",
        "https://api.example.com/v1/query",
        "-H",
        "MalformedHeaderWithoutColon",
    ])
    .unwrap();

    let err = BenchmarkConfig::from_cli(cli).unwrap_err();
    assert!(matches!(err, ConfigError::InvalidHeader(_)));
}

#[test]
fn test_body_inline_vs_file_resolution() {
    // 1. Inline raw string body
    let cli_inline = Cli::try_parse_from([
        "rusty-loader",
        "-u",
        "https://api.example.com/v1/query",
        "-m",
        "POST",
        "-b",
        r#"{"query":"active_records","limit":50}"#,
    ])
    .unwrap();

    let config_inline = BenchmarkConfig::from_cli(cli_inline).unwrap();
    assert_eq!(
        config_inline.body.as_deref(),
        Some(r#"{"query":"active_records","limit":50}"#)
    );
    assert_eq!(
        config_inline.body_bytes(),
        Some(br#"{"query":"active_records","limit":50}"#.as_slice())
    );

    // 2. File body prefixed with '@'
    let mut temp_file = NamedTempFile::new().expect("failed to create temp file");
    let file_payload = r#"{"from_file": true, "items": [1, 2, 3]}"#;
    write!(temp_file, "{file_payload}").expect("failed to write temp payload");

    let at_arg = format!("@{}", temp_file.path().display());
    let cli_file = Cli::try_parse_from([
        "rusty-loader",
        "-u",
        "https://api.example.com/v1/query",
        "-m",
        "POST",
        "-b",
        &at_arg,
    ])
    .unwrap();

    let config_file = BenchmarkConfig::from_cli(cli_file).unwrap();
    assert_eq!(config_file.body.as_deref(), Some(file_payload));

    // 3. Missing file error rejection
    let cli_missing = Cli::try_parse_from([
        "rusty-loader",
        "-u",
        "https://api.example.com/v1/query",
        "-b",
        "@/nonexistent/path/to/payload_12345.json",
    ])
    .unwrap();

    let err = BenchmarkConfig::from_cli(cli_missing).unwrap_err();
    assert!(matches!(err, ConfigError::BodyFileReadError { .. }));
}

#[test]
fn test_invalid_url_rejection() {
    // Unparseable URL
    let cli_bad_url = Cli::try_parse_from(["rusty-loader", "-u", "not a valid url"]).unwrap();
    let err = BenchmarkConfig::from_cli(cli_bad_url).unwrap_err();
    assert!(matches!(err, ConfigError::InvalidUrl { .. }));

    // Unsupported scheme
    let cli_ftp = Cli::try_parse_from(["rusty-loader", "-u", "ftp://files.example.com/test"]).unwrap();
    let err_ftp = BenchmarkConfig::from_cli(cli_ftp).unwrap_err();
    assert!(matches!(err_ftp, ConfigError::UnsupportedScheme { .. }));
}

#[test]
fn test_ramp_up_and_bounds_validation() {
    // Valid: ramp_up (10s) <= duration (120s), plus output-json
    let cli_valid = Cli::try_parse_from([
        "api-load-tester",
        "--url",
        "https://api.example.com/v1/query",
        "--method",
        "POST",
        "--concurrency",
        "700",
        "--ramp-up",
        "10",
        "--duration",
        "120s",
        "--output-json",
        "report.json",
    ])
    .unwrap();

    let config = BenchmarkConfig::from_cli(cli_valid).unwrap();
    assert_eq!(config.ramp_up, Duration::from_secs(10));
    assert_eq!(config.duration, Duration::from_secs(120));
    assert_eq!(config.output_json, Some(PathBuf::from("report.json")));

    // Invalid: ramp_up (60s) > duration (30s)
    let cli_exceed = Cli::try_parse_from([
        "rusty-loader",
        "--url",
        "https://api.example.com/v1/ping",
        "--duration",
        "30s",
        "--ramp-up",
        "60s",
    ])
    .unwrap();

    let err = BenchmarkConfig::from_cli(cli_exceed).unwrap_err();
    assert!(matches!(err, ConfigError::RampUpExceedsDuration { .. }));

    // Invalid: concurrency == 0
    let cli_zero_conc = Cli::try_parse_from([
        "rusty-loader",
        "--url",
        "https://api.example.com/v1/ping",
        "--concurrency",
        "0",
    ])
    .unwrap();
    assert!(matches!(
        BenchmarkConfig::from_cli(cli_zero_conc).unwrap_err(),
        ConfigError::InvalidConcurrency(0)
    ));

    // Invalid: duration == 0
    let cli_zero_dur = Cli::try_parse_from([
        "rusty-loader",
        "--url",
        "https://api.example.com/v1/ping",
        "--duration",
        "0s",
    ])
    .unwrap();
    assert!(matches!(
        BenchmarkConfig::from_cli(cli_zero_dur).unwrap_err(),
        ConfigError::InvalidDuration
    ));
}
