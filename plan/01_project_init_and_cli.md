# Task 1: Project Setup, Dependencies & CLI Specification

## 1. Goal & Context
Initialize the Cargo package for `rusty-loader` (with binary aliases if needed for `api-load-tester`), configure all production and testing dependencies, and implement the command-line argument parser and runtime configuration layer using `clap` derive.

---

## 2. Target Files
- `Cargo.toml`: Package metadata, binary targets, dependencies, profile optimizations.
- `src/main.rs`: Minimal entrypoint dispatching to CLI parsing.
- `src/lib.rs`: Root library file declaring modules (`cli`, `config`, `client`, `engine`, `metrics`, `reporter`, `os`).
- `src/cli.rs`: Command-line interface definitions with `clap` derive macros.
- `src/config.rs`: Strongly typed runtime configuration, validations, and conversions.
- `tests/cli_parsing_test.rs`: Unit and integration tests for CLI argument parsing.

---

## 3. Detailed Specifications

### 3.1 Cargo.toml Configuration
```toml
[package]
name = "rusty-loader"
version = "0.1.0"
edition = "2021"
authors = ["Sean Rafferty"]
description = "High-performance, concurrent API load testing CLI written in Rust"

[[bin]]
name = "rusty-loader"
path = "src/main.rs"

[[bin]]
name = "api-load-tester"
path = "src/main.rs"

[dependencies]
# Async Runtime & Networking
tokio = { version = "1.40", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "stream"] }

# CLI Parsing & UX
clap = { version = "4.5", features = ["derive", "env"] }
indicatif = { version = "0.17", features = ["tokio"] }
comfy-table = "7.1"
colored = "2.1"

# Telemetry & Metrics
hdrhistogram = "7.5"
chrono = "0.4"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Error Handling & Utilities
anyhow = "1.0"
thiserror = "1.0"

[dev-dependencies]
wiremock = "0.6"
tokio-test = "0.4"
tempfile = "3.12"

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

### 3.2 CLI Arguments (`src/cli.rs`)
The CLI must accept the following flags and options:
- `--url` (`-u`): Target endpoint (e.g. `https://api.example.com/v1/ping`, required).
- `--concurrency` (`-c`): Number of concurrent workers (default: `700`).
- `--duration` (`-d`): Benchmark duration (default: `30s`, accepts strings like `30s`, `2m`, `1000ms`, or raw seconds).
- `--method` (`-m`): HTTP method (default: `GET`, accepts `GET`, `POST`, `PUT`, `DELETE`, `PATCH`, `HEAD`).
- `--header` (`-H`): Key-value headers, repeatable (e.g. `-H "Authorization: Bearer xyz"`).
- `--body` (`-b`): Optional body as raw string or payload file path prefixed with `@` (e.g. `@payload.json`).
- `--timeout` (`-t`): Request timeout in milliseconds (default: `5000ms`, also parses duration strings).
- `--ramp-up`: Optional ramp-up duration in seconds or duration string (default: `0s`).
- `--output-json`: Optional path to output final JSON report.

### 3.3 Runtime Configuration (`src/config.rs`)
Convert `Cli` into a validated `BenchmarkConfig`:
- Validate URL scheme is `http` or `https`.
- Ensure concurrency $> 0$.
- Ensure duration $> 0$.
- Parse headers into `reqwest::header::HeaderMap`.
- If body starts with `@`, load contents from disk (with descriptive error if file missing).
- Validate ramp-up duration is $\le$ total benchmark duration.

---

## 4. Verification & Testing

### Automated Tests
Run:
```bash
cargo check
cargo test --test cli_parsing_test
```

### Test Cases in `tests/cli_parsing_test.rs`:
1. Default values (`concurrency = 700`, `duration = 30s`, `timeout = 5000ms`, `method = GET`).
2. Duration string parsing (`30s`, `2m`, `500ms`, `60`).
3. Repeatable headers parsing into `HeaderMap`.
4. Body file resolution (`@payload.json`) vs inline raw body string.
5. Invalid URL error rejection.
6. Validation that ramp-up cannot exceed total duration.

---

## 5. Sub-Agent Prompt Template

```text
You are executing Task 1 of the rusty-loader project.
Review plan/01_project_init_and_cli.md for instructions.
Tasks to perform:
1. Initialize Cargo.toml with all specified dependencies and release profile.
2. Implement src/cli.rs with clap derive and custom duration/header parsing helpers.
3. Implement src/config.rs with BenchmarkConfig struct and validation logic.
4. Expose modules in src/lib.rs and wire basic arg parsing in src/main.rs.
5. Create tests/cli_parsing_test.rs covering defaults, duration formats, headers, and body loading.
6. Run `cargo check` and `cargo test --test cli_parsing_test` to verify everything builds and passes.
```

---

## 6. Acceptance Criteria Checklist
- [ ] `Cargo.toml` contains all dependencies and builds cleanly.
- [ ] Both binary names `rusty-loader` and `api-load-tester` are configured.
- [ ] CLI parses all flags matching the issue specification.
- [ ] Duration string parser cleanly handles `s`, `m`, `ms`, and plain integers.
- [ ] Headers `-H` parse into `HeaderMap` rejecting malformed headers with friendly errors.
- [ ] All unit and integration tests in `cli_parsing_test.rs` pass.
