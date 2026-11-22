//! When the checks run: one minute after the start, then every `check_interval_hours`.

use std::time::Duration;

/// The first check waits for the start to settle.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);

/// `update.check_interval_hours` (at least 1, checked by the configuration).
pub fn check_interval(hours: u32) -> Duration {
    Duration::from_secs(u64::from(hours.max(1)) * 3600)
}
