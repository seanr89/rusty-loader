use crate::metrics::stats::LatencyDistribution;
use std::time::Duration;

/// Minimum measurable latency in microseconds (1 µs).
pub const MIN_LATENCY_MICROS: u64 = 1;
/// Maximum measurable latency in microseconds (60 seconds).
pub const MAX_LATENCY_MICROS: u64 = 60_000_000;
/// Number of significant value digits maintained by the HDR histogram.
pub const HISTOGRAM_SIGFIGS: u8 = 3;

/// Wrapper around `hdrhistogram::Histogram<u64>` recording latencies in microseconds.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    histogram: hdrhistogram::Histogram<u64>,
}

impl LatencyHistogram {
    /// Creates a new `LatencyHistogram` bounded from 1 µs to 60,000,000 µs (60s) with 3 significant figures.
    pub fn new() -> Result<Self, hdrhistogram::CreationError> {
        let histogram = hdrhistogram::Histogram::<u64>::new_with_bounds(
            MIN_LATENCY_MICROS,
            MAX_LATENCY_MICROS,
            HISTOGRAM_SIGFIGS,
        )?;
        Ok(Self { histogram })
    }

    /// Records a latency duration, clamping values outside `[1 µs, 60 s]` to the boundary limits.
    pub fn record(&mut self, latency: Duration) {
        let micros = latency
            .as_micros()
            .clamp(MIN_LATENCY_MICROS as u128, MAX_LATENCY_MICROS as u128) as u64;
        let _ = self.histogram.record(micros);
    }

    /// Returns the total number of recorded latency samples.
    pub fn len(&self) -> u64 {
        self.histogram.len()
    }

    /// Returns `true` if no samples have been recorded.
    pub fn is_empty(&self) -> bool {
        self.histogram.is_empty()
    }

    /// Queries the latency at the given percentile `p` (in `0.0..=100.0`) as a `Duration`.
    pub fn percentile(&self, p: f64) -> Duration {
        if self.histogram.is_empty() {
            return Duration::ZERO;
        }
        Duration::from_micros(self.histogram.value_at_percentile(p))
    }

    /// Queries the latency at the given percentile `p` (in `0.0..=100.0`) in milliseconds.
    pub fn percentile_ms(&self, p: f64) -> f64 {
        if self.histogram.is_empty() {
            return 0.0;
        }
        self.histogram.value_at_percentile(p) as f64 / 1_000.0
    }

    /// Returns the minimum recorded latency in milliseconds, or `0.0` if empty.
    pub fn min_ms(&self) -> f64 {
        if self.histogram.is_empty() {
            return 0.0;
        }
        self.histogram.min() as f64 / 1_000.0
    }

    /// Returns the maximum recorded latency in milliseconds, or `0.0` if empty.
    pub fn max_ms(&self) -> f64 {
        if self.histogram.is_empty() {
            return 0.0;
        }
        self.histogram.max() as f64 / 1_000.0
    }

    /// Returns the mean latency in milliseconds, or `0.0` if empty.
    pub fn mean_ms(&self) -> f64 {
        if self.histogram.is_empty() {
            return 0.0;
        }
        self.histogram.mean() / 1_000.0
    }

    /// Returns the standard deviation of latencies in milliseconds, or `0.0` if empty.
    pub fn std_dev_ms(&self) -> f64 {
        if self.histogram.is_empty() {
            return 0.0;
        }
        self.histogram.stdev() / 1_000.0
    }

    /// Exports a full [`LatencyDistribution`] snapshot in milliseconds.
    pub fn to_distribution(&self) -> LatencyDistribution {
        if self.histogram.is_empty() {
            return LatencyDistribution::default();
        }
        LatencyDistribution {
            min_ms: self.min_ms(),
            mean_ms: self.mean_ms(),
            p50_ms: self.percentile_ms(50.0),
            p90_ms: self.percentile_ms(90.0),
            p95_ms: self.percentile_ms(95.0),
            p99_ms: self.percentile_ms(99.0),
            p99_9_ms: self.percentile_ms(99.9),
            max_ms: self.max_ms(),
            std_dev_ms: self.std_dev_ms(),
        }
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new().expect("valid constant HDR histogram bounds")
    }
}
