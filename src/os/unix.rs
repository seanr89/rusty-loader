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
