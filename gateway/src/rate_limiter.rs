// Rate limiter: token bucket implementation

use parking_lot::Mutex;
use std::time::Instant;

/// RateLimiter: token bucket rate limiter
pub struct RateLimiter {
    capacity: u32,
    refill_rate: f64,      // tokens per second
    tokens: Mutex<f64>,
    last_refill: Mutex<Instant>,
}

impl RateLimiter {
    pub fn new(rps: u32) -> Self {
        Self {
            capacity: rps,
            refill_rate: rps as f64,
            tokens: Mutex::new(rps as f64),
            last_refill: Mutex::new(Instant::now()),
        }
    }

    /// Check if a request is allowed
    pub fn allow_request(&self) -> bool {
        let mut tokens = self.tokens.lock();
        let mut last_refill = self.last_refill.lock();

        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill).as_secs_f64();
        *tokens = (*tokens + elapsed * self.refill_rate).min(self.capacity as f64);
        *last_refill = now;

        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Get current token count
    pub fn current_tokens(&self) -> f64 {
        *self.tokens.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_rate_limiter() {
        let limiter = RateLimiter::new(10); // 10 RPS

        // Should allow 10 requests immediately
        for _ in 0..10 {
            assert!(limiter.allow_request());
        }

        // 11th should be denied
        assert!(!limiter.allow_request());

        // Wait 1 second, should have refilled ~10 tokens
        thread::sleep(Duration::from_millis(1100));

        // Should allow more requests
        assert!(limiter.allow_request());
    }
}
