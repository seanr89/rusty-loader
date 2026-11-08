use reqwest::StatusCode;
use rusty_loader::metrics::{
     spawn_collector, BenchmarkStats, LatencyDistribution, LatencyHistogram, LiveCounters,
    MetricEvent, MetricsCollector,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[test]
fn test_histogram_accuracy_10000_samples() {
    let mut hist = LatencyHistogram::new().expect("histogram creation should succeed");
    assert!(hist.is_empty());
    assert_eq!(hist.len(), 0);

    // Record 10,000 uniform samples from 1 µs to 10,000 µs (0.001 ms to 10.000 ms)
    for micros in 1..=10_000 {
        hist.record(Duration::from_micros(micros));
    }

    assert!(!hist.is_empty());
    assert_eq!(hist.len(), 10_000);

    let dist = hist.to_distribution();

    // With 3 significant figures of precision, HDR histogram values are accurate within ~0.1%
    assert!(
        (dist.min_ms - 0.001).abs() < 0.001,
        "expected min ~0.001 ms, got {}",
        dist.min_ms
    );
    assert!(
        (dist.max_ms - 10.0).abs() <= 0.02,
        "expected max ~10.0 ms, got {}",
        dist.max_ms
    );
    assert!(
        (dist.mean_ms - 5.0005).abs() <= 0.05,
        "expected mean ~5.0 ms, got {}",
        dist.mean_ms
    );
    assert!(
        (dist.p50_ms - 5.0).abs() <= 0.05,
        "expected p50 ~5.0 ms, got {}",
        dist.p50_ms
    );
    assert!(
        (dist.p90_ms - 9.0).abs() <= 0.05,
        "expected p90 ~9.0 ms, got {}",
        dist.p90_ms
    );
    assert!(
        (dist.p95_ms - 9.5).abs() <= 0.05,
        "expected p95 ~9.5 ms, got {}",
        dist.p95_ms
    );
    assert!(
        (dist.p99_ms - 9.9).abs() <= 0.05,
        "expected p99 ~9.9 ms, got {}",
        dist.p99_ms
    );
    assert!(
        (dist.p99_9_ms - 9.99).abs() <= 0.05,
        "expected p99.9 ~9.99 ms, got {}",
        dist.p99_9_ms
    );
    assert!(
        dist.std_dev_ms > 2.8 && dist.std_dev_ms < 3.0,
        "expected std_dev ~2.886 ms, got {}",
        dist.std_dev_ms
    );

    // Also verify Duration-returning percentile method
    let p50_dur = hist.percentile(50.0);
    assert!(
        (p50_dur.as_micros() as i64 - 5_000).abs() <= 50,
        "expected p50 duration ~5000 µs, got {:?}",
        p50_dur
    );
}

#[test]
fn test_histogram_boundary_clamping_and_empty_state() {
    let mut hist = LatencyHistogram::new().expect("histogram creation should succeed");

    // Empty histogram should safely return 0 for all metrics
    assert_eq!(hist.percentile(50.0), Duration::ZERO);
    assert_eq!(hist.to_distribution(), LatencyDistribution::default());

    // Record sub-microsecond latency (< 1 µs): should clamp to 1 µs (0.001 ms)
    hist.record(Duration::ZERO);
    hist.record(Duration::from_nanos(250));

    // Record latency exceeding 60 seconds (> 60,000,000 µs): should clamp to 60s (60,000 ms)
    hist.record(Duration::from_secs(120));
    hist.record(Duration::from_secs(3600));

    assert_eq!(hist.len(), 4);
    assert!(
        (hist.min_ms() - 0.001).abs() < 1e-6,
        "sub-microsecond latency should clamp to 1 µs (0.001 ms), got {}",
        hist.min_ms()
    );
    // HDR histogram with 3 sig figs may quantize 60_000_000 slightly (within 0.1%)
    assert!(
        (hist.max_ms() - 60_000.0).abs() <= 100.0,
        "over-60s latency should clamp to ~60,000 ms, got {}",
        hist.max_ms()
    );
}

#[tokio::test]
async fn test_status_code_counters_and_channel_collector() {
    let (tx, rx) = mpsc::unbounded_channel();
    let counters = Arc::new(LiveCounters::new());
    let start = Instant::now();

    let collector_handle = spawn_collector(rx, Arc::clone(&counters), start);

    // 2xx successful responses
    tx.send(MetricEvent::response(
        Duration::from_millis(10),
        StatusCode::OK,
        100,
    ))
    .unwrap();
    tx.send(MetricEvent::response(
        Duration::from_millis(15),
        StatusCode::OK,
        250,
    ))
    .unwrap();
    tx.send(MetricEvent::response(
        Duration::from_millis(20),
        StatusCode::CREATED,
        50,
    ))
    .unwrap();

    // 3xx other status
    tx.send(MetricEvent::response(
        Duration::from_millis(5),
        StatusCode::FOUND,
        0,
    ))
    .unwrap();

    // 4xx client errors
    tx.send(MetricEvent::response(
        Duration::from_millis(12),
        StatusCode::NOT_FOUND,
        40,
    ))
    .unwrap();
    tx.send(MetricEvent::response(
        Duration::from_millis(8),
        StatusCode::BAD_REQUEST,
        60,
    ))
    .unwrap();

    // 5xx server errors
    tx.send(MetricEvent::response(
        Duration::from_millis(30),
        StatusCode::INTERNAL_SERVER_ERROR,
        80,
    ))
    .unwrap();

    // Transport / network errors
    tx.send(MetricEvent::error(Duration::from_millis(1000), "Timeout"))
        .unwrap();
    tx.send(MetricEvent::error(Duration::from_millis(1000), "Timeout"))
        .unwrap();
    tx.send(MetricEvent::error(
        Duration::from_millis(5),
        "ConnectError",
    ))
    .unwrap();

    // Drop sender so collector finishes
    drop(tx);

    let stats = collector_handle
        .await
        .expect("collector task should complete cleanly");

    // Verify LiveCounters match final stats
    assert_eq!(counters.total_requests(), 10);
    assert_eq!(counters.successful_requests(), 3);
    assert_eq!(counters.other_statuses(), 1);
    assert_eq!(counters.client_errors(), 2);
    assert_eq!(counters.server_errors(), 1);
    assert_eq!(counters.transport_errors(), 3);
    assert_eq!(counters.total_errors(), 6); // 2 client + 1 server + 3 transport
    assert_eq!(counters.total_bytes(), 580);

    // Verify BenchmarkStats fields
    assert_eq!(stats.total_requests, 10);
    assert_eq!(stats.successful_requests, 3);
    assert_eq!(stats.other_statuses, 1);
    assert_eq!(stats.client_errors, 2);
    assert_eq!(stats.server_errors, 1);
    assert_eq!(stats.transport_errors, 3);
    assert_eq!(stats.total_bytes, 580);
    assert!(stats.total_duration_secs > 0.0);
    assert!(stats.requests_per_sec > 0.0);
    assert!(stats.bytes_per_sec > 0.0);

    // Verify status distribution map
    assert_eq!(stats.status_distribution.get(&200), Some(&2));
    assert_eq!(stats.status_distribution.get(&201), Some(&1));
    assert_eq!(stats.status_distribution.get(&302), Some(&1));
    assert_eq!(stats.status_distribution.get(&400), Some(&1));
    assert_eq!(stats.status_distribution.get(&404), Some(&1));
    assert_eq!(stats.status_distribution.get(&500), Some(&1));

    // Verify error breakdown map
    assert_eq!(stats.error_breakdown.get("Timeout"), Some(&2));
    assert_eq!(stats.error_breakdown.get("ConnectError"), Some(&1));
}

#[test]
fn test_benchmark_stats_json_serialization_roundtrip() {
    let mut status_distribution = HashMap::new();
    status_distribution.insert(200, 9950);
    status_distribution.insert(404, 30);
    status_distribution.insert(500, 15);

    let mut error_breakdown = HashMap::new();
    error_breakdown.insert("Timeout".to_string(), 4);
    error_breakdown.insert("ConnectError".to_string(), 1);

    let original = BenchmarkStats {
        total_duration_secs: 10.0,
        total_requests: 10_000,
        successful_requests: 9950,
        client_errors: 30,
        server_errors: 15,
        other_statuses: 0,
        transport_errors: 5,
        requests_per_sec: 1000.0,
        total_bytes: 2_500_000,
        bytes_per_sec: 250_000.0,
        latencies: LatencyDistribution {
            min_ms: 0.45,
            mean_ms: 4.21,
            p50_ms: 3.80,
            p90_ms: 7.10,
            p95_ms: 11.40,
            p99_ms: 18.90,
            p99_9_ms: 45.20,
            max_ms: 120.50,
            std_dev_ms: 3.15,
        },
        status_distribution,
        error_breakdown,
    };

    let json = serde_json::to_string_pretty(&original).expect("serialization should succeed");
    assert!(json.contains("\"total_requests\": 10000"));
    assert!(json.contains("\"p99_ms\": 18.9"));

    let deserialized: BenchmarkStats =
        serde_json::from_str(&json).expect("deserialization should succeed");
    assert_eq!(original, deserialized);
}

#[test]
fn test_collector_finalize_with_zero_duration() {
    let mut collector = MetricsCollector::default();
    collector.record(MetricEvent::success(
        Duration::from_millis(4),
        StatusCode::OK,
        128,
    ));

    let stats = collector.finalize(Duration::ZERO);
    assert_eq!(stats.total_requests, 1);
    assert_eq!(stats.requests_per_sec, 0.0);
    assert_eq!(stats.bytes_per_sec, 0.0);
}
