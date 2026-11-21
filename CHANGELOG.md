# Changelog

All notable changes to `rusty-loader` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-21

### Features
- automate versioning on main merges, embed CHANGELOG, and add About screen (#5) (`ca320e0`)
- **gui**: add embedded real-time web monitoring dashboard module and update lib structure (`e62c5b7`)
- **gui**: implement real-time embedded web monitoring dashboard (#3) (`61db939`)
- implement rusty-loader high-performance API load testing CLI (`bf8bb07`)

### Maintenance
- Initial commit (`8818c22`)

## [0.1.0] - 2026-11-21

### Features
- Automated Semantic Versioning and `CHANGELOG.md` updates on merge to `main`
- Embedded `CHANGELOG.md` in binary (`rusty-loader --changelog`) for offline release notes inspection
- Added in-app About modal dialog and `/api/about` + `/api/changelog` endpoints in the GUI dashboard
- Real-time embedded web monitoring dashboard (`--gui`) with SSE telemetry streaming, adaptive light/dark theme, and live charts
- High-performance concurrent HTTP load testing engine using Tokio and Reqwest (`rustls-tls`)
- Per-worker HDR latency histograms (`p50`, `p90`, `p95`, `p99`, `p99.9`, `max`, `mean`, `std_dev`) and lock-free atomic live counters
- Terminal progress telemetry and formatted summary tables with optional JSON report export (`--output-json`)
