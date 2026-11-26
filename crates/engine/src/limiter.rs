use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Global token-bucket speed limiter shared by every connection.
/// A rate of 0 means unlimited.
pub(crate) struct Limiter {
    rate: AtomicU64,
    state: Mutex<(f64, Instant)>,
}

impl Limiter {
    pub fn new(rate: u64) -> Self {
        Self {
            rate: AtomicU64::new(rate),
            state: Mutex::new((0.0, Instant::now())),
        }
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Relaxed)
    }

    pub fn set_rate(&self, rate: u64) {
        self.rate.store(rate, Relaxed);
    }

    /// Accounts for `n` bytes, sleeping when the budget is overdrawn.
    pub async fn acquire(&self, n: usize) {
        let rate = self.rate.load(Relaxed);
        if rate == 0 {
            return;
        }
        let rate = rate as f64;
        let wait = {
            let mut s = self.state.lock().unwrap();
            let now = Instant::now();
            let refill = now.duration_since(s.1).as_secs_f64() * rate;
            s.0 = (s.0 + refill).min(rate * 0.5);
            s.1 = now;
            s.0 -= n as f64;
            if s.0 < 0.0 {
                -s.0 / rate
            } else {
                0.0
            }
        };
        if wait > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}
