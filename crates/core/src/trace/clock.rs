use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

/// Wall-clock time of the journal: entry times, log file rotation and retention, archive names. Injectable so that
/// rotation and retention can be tested without waiting.
pub trait WallClock: Send + Sync {
    fn now(&self) -> SystemTime;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A clock that only moves when told to. Clones share the same time.
#[derive(Debug, Clone)]
pub struct ManualWallClock(Arc<Mutex<SystemTime>>);

impl ManualWallClock {
    pub fn new(at: SystemTime) -> Self {
        Self(Arc::new(Mutex::new(at)))
    }

    pub fn set(&self, at: SystemTime) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = at;
    }

    pub fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now += by;
    }
}

impl WallClock for ManualWallClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
