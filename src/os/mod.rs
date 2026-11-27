//! OS sanity checks and file descriptor limit inspection.

use serde::{Deserialize, Serialize};

#[cfg(unix)]
pub mod unix;

/// Recommended minimum file descriptor soft limit for high-concurrency workloads.
pub const RECOMMENDED_MIN_NOFILE: u64 = 4096;

/// Structured representation of the operating system's open file descriptor limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsLimits {
    /// Current soft limit (`rlim_cur` on Unix).
    pub soft_limit: u64,
    /// Current hard limit (`rlim_max` on Unix).
    pub hard_limit: u64,
    /// Recommended minimum soft limit (`4096`).
    pub recommended_min: u64,
}

impl OsLimits {
    /// Queries the current OS file descriptor limits without printing any warning.
    pub fn detect() -> Self {
        #[cfg(unix)]
        {
            unix::get_rlimit_nofile()
        }
        #[cfg(not(unix))]
        {
            Self {
                soft_limit: u64::MAX,
                hard_limit: u64::MAX,
                recommended_min: RECOMMENDED_MIN_NOFILE,
            }
        }
    }

    /// Returns `true` if the current soft limit meets or exceeds `recommended_min`.
    pub fn is_sufficient(&self) -> bool {
        self.soft_limit >= self.recommended_min
    }

    /// Formats a warning message if the soft limit is below `recommended_min`, or `None` if sufficient.
    pub fn warning_message(&self) -> Option<String> {
        if self.is_sufficient() {
            None
        } else {
            Some(format!(
                "Warning: OS file descriptor limit is {} (recommended >= {}). Run 'ulimit -n 65536' before testing.",
                self.soft_limit, self.recommended_min
            ))
        }
    }
}

/// Queries the current OS file descriptor limits without printing warnings.
pub fn get_os_limits() -> OsLimits {
    OsLimits::detect()
}

/// Best-effort attempt to raise the process file descriptor soft limit up to `preferred`.
///
/// On Unix/macOS, clamps against the hard limit and falls back cleanly without failing if the OS
/// rejects higher ceilings. On non-Unix platforms, returns [`OsLimits::detect`].
pub fn raise_nofile_soft_limit(preferred: u64) -> OsLimits {
    #[cfg(unix)]
    {
        unix::raise_nofile_soft_limit(preferred)
    }
    #[cfg(not(unix))]
    {
        let _ = preferred;
        OsLimits::detect()
    }
}

/// Checks OS file descriptor limits and prints a visible warning to stderr if `soft_limit < 4096`.
pub fn check_os_limits() -> OsLimits {
    let limits = OsLimits::detect();
    if let Some(warning) = limits.warning_message() {
        eprintln!("{warning}");
    }
    limits
}

/// Alias for [`check_os_limits`].
pub fn check_and_warn_os_limits() -> OsLimits {
    check_os_limits()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_os_limits_warning_when_low() {
        let low = OsLimits {
            soft_limit: 256,
            hard_limit: 65536,
            recommended_min: RECOMMENDED_MIN_NOFILE,
        };
        assert!(!low.is_sufficient());
        assert_eq!(
            low.warning_message().as_deref(),
            Some("Warning: OS file descriptor limit is 256 (recommended >= 4096). Run 'ulimit -n 65536' before testing.")
        );
    }

    #[test]
    fn test_os_limits_sufficient() {
        let ok = OsLimits {
            soft_limit: 4096,
            hard_limit: 65536,
            recommended_min: RECOMMENDED_MIN_NOFILE,
        };
        assert!(ok.is_sufficient());
        assert!(ok.warning_message().is_none());
    }

    #[test]
    fn test_detect_os_limits() {
        let limits = check_os_limits();
        assert_eq!(limits.recommended_min, 4096);
        assert!(limits.soft_limit > 0);
    }

    #[test]
    fn test_raise_nofile_soft_limit_non_fatal() {
        let before = get_os_limits();
        let after = raise_nofile_soft_limit(65_536);
        assert!(after.soft_limit >= before.soft_limit || after.soft_limit > 0);
        assert_eq!(after.recommended_min, RECOMMENDED_MIN_NOFILE);
    }
}
