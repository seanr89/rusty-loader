//! Unix/macOS file descriptor limit (`rlimit` / `ulimit -n`) inspection.

use super::{OsLimits, RECOMMENDED_MIN_NOFILE};

/// Queries the current Unix process file descriptor limits (`RLIMIT_NOFILE`).
pub fn get_rlimit_nofile() -> OsLimits {
    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    // SAFETY: Passing a valid mutable pointer to `libc::rlimit` for `RLIMIT_NOFILE`.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) };
    if rc == 0 {
        OsLimits {
            soft_limit: rlim.rlim_cur as u64,
            hard_limit: rlim.rlim_max as u64,
            recommended_min: RECOMMENDED_MIN_NOFILE,
        }
    } else {
        // Fallback if getrlimit unexpectedly fails
        OsLimits {
            soft_limit: RECOMMENDED_MIN_NOFILE,
            hard_limit: RECOMMENDED_MIN_NOFILE,
            recommended_min: RECOMMENDED_MIN_NOFILE,
        }
    }
}

/// Best-effort attempt to raise the Unix process `RLIMIT_NOFILE` soft limit up to `preferred`.
///
/// Useful on macOS when `.app` bundles launched by LaunchServices start with a low soft limit
/// (commonly `256`). Clamps against `rlim_max` and falls back through standard macOS/Unix ceilings
/// (`preferred`, `65536`, `10240`, `4096`) without failing startup if `setrlimit` rejects higher values.
pub fn raise_nofile_soft_limit(preferred: u64) -> OsLimits {
    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    // SAFETY: Passing a valid mutable pointer to `libc::rlimit` for `RLIMIT_NOFILE`.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) } != 0 {
        return get_rlimit_nofile();
    }

    let current = rlim.rlim_cur as u64;
    if current >= preferred {
        return get_rlimit_nofile();
    }

    let max_cap = if rlim.rlim_max == libc::RLIM_INFINITY {
        u64::MAX
    } else {
        rlim.rlim_max as u64
    };

    let candidates = [preferred, 65_536, 10_240, RECOMMENDED_MIN_NOFILE];
    for candidate in candidates {
        let target = candidate.min(max_cap);
        if target <= current {
            continue;
        }
        let next = libc::rlimit {
            rlim_cur: target as libc::rlim_t,
            rlim_max: rlim.rlim_max,
        };
        // SAFETY: Passing a valid pointer to `libc::rlimit` for `RLIMIT_NOFILE`.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &next) } == 0 {
            break;
        }
    }

    get_rlimit_nofile()
}
