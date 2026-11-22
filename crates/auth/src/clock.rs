use std::time::SystemTime;

use mcp_proxy_core::trace::WallClock;
use tokio::time::Instant;

/// Wall-clock time that moves with tokio's clock: the system time at creation, plus the time tokio has seen pass
/// since. Under a paused tokio clock (tests), expiries, the margin and the timers move together with
/// `tokio::time::advance`. The application uses `SystemWallClock`: after a sleep of the computer, the wall clock is
/// right and the timers late, and expiry is checked against the wall clock at every use.
#[derive(Debug, Clone, Copy)]
pub struct TokioWallClock {
    wall: SystemTime,
    instant: Instant,
}

impl TokioWallClock {
    pub fn new() -> Self {
        Self::starting_at(SystemTime::now())
    }

    /// Starts at `wall` now.
    pub fn starting_at(wall: SystemTime) -> Self {
        Self {
            wall,
            instant: Instant::now(),
        }
    }
}

impl Default for TokioWallClock {
    fn default() -> Self {
        Self::new()
    }
}

impl WallClock for TokioWallClock {
    fn now(&self) -> SystemTime {
        let elapsed = Instant::now().saturating_duration_since(self.instant);
        self.wall.checked_add(elapsed).unwrap_or(self.wall)
    }
}
