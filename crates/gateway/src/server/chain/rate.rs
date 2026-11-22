//! One token bucket per instance: `burst` tokens at most, refilled at `per_minute` tokens a minute.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use mcp_proxy_core::config::RateLimit;

use crate::registry::InstanceId;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct RateLimiter {
    buckets: Mutex<HashMap<InstanceId, Bucket>>,
}

impl RateLimiter {
    /// Takes one token, or says how long until one is available. Nothing is taken on refusal.
    pub(crate) fn acquire(&self, instance: &InstanceId, limit: &RateLimit, now: Instant) -> Result<(), Duration> {
        let capacity = f64::from(limit.burst.max(1));
        let per_second = f64::from(limit.per_minute.max(1)) / 60.0;
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        let bucket = buckets.entry(instance.clone()).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * per_second).min(capacity);
        bucket.updated = now.max(bucket.updated);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / per_second))
        }
    }
}

#[cfg(test)]
mod tests {
    use mcp_proxy_core::config::{EnvId, McpId};

    use super::*;

    fn instance() -> InstanceId {
        InstanceId::new(EnvId::new("prod").unwrap(), McpId::new("kibana-logs").unwrap())
    }

    // Exigence : EX-007-03
    #[test]
    fn burst_then_refill_at_the_configured_rate() {
        let limiter = RateLimiter::default();
        let limit = RateLimit {
            per_minute: 60,
            burst: 3,
        };
        let start = Instant::now();
        for _ in 0..3 {
            limiter.acquire(&instance(), &limit, start).unwrap();
        }
        let wait = limiter.acquire(&instance(), &limit, start).unwrap_err();
        assert!(
            wait > Duration::from_millis(990) && wait <= Duration::from_secs(1),
            "{wait:?}"
        );
        // A refusal takes nothing: half a second later, half a token is still missing.
        let half = start + Duration::from_millis(500);
        let wait = limiter.acquire(&instance(), &limit, half).unwrap_err();
        assert!(
            wait > Duration::from_millis(490) && wait <= Duration::from_millis(500),
            "{wait:?}"
        );
        limiter
            .acquire(&instance(), &limit, start + Duration::from_secs(1))
            .unwrap();
        // Never more than `burst` after a long pause.
        let later = start + Duration::from_secs(3600);
        for _ in 0..3 {
            limiter.acquire(&instance(), &limit, later).unwrap();
        }
        assert!(limiter.acquire(&instance(), &limit, later).is_err());
        // Buckets are per instance.
        let other = InstanceId::new(EnvId::new("dev").unwrap(), McpId::new("kibana-logs").unwrap());
        limiter.acquire(&other, &limit, later).unwrap();
    }
}
