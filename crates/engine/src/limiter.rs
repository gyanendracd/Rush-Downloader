use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Token-bucket speed limiter. A rate of 0 means unlimited.
///
/// The engine has one shared by every connection of every download (the
/// global limit, Settings), and each running download has one of its own
/// whose `parent` is that one: bytes are let through only when both allow
/// them, so a download keeps to its own limit and all of them together keep
/// to the global one.
pub(crate) struct Limiter {
    rate: AtomicU64,
    state: Mutex<(f64, Instant)>,
    parent: Option<Arc<Limiter>>,
}

impl Limiter {
    pub fn new(rate: u64) -> Self {
        Self {
            rate: AtomicU64::new(rate),
            state: Mutex::new((0.0, Instant::now())),
            parent: None,
        }
    }

    /// A download's own limiter, inside the global one.
    pub fn under(parent: Arc<Limiter>, rate: u64) -> Self {
        Self { parent: Some(parent), ..Self::new(rate) }
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Relaxed)
    }

    /// The speed that applies here: the lower of this limit and the ones
    /// above it; 0 when none is set. For programs that are given a rate
    /// rather than run through `acquire` (yt-dlp, gallery-dl, the torrent
    /// library).
    pub fn effective_rate(&self) -> u64 {
        let own = self.rate();
        let above = self.parent.as_ref().map_or(0, |p| p.effective_rate());
        match (own, above) {
            (0, r) | (r, 0) => r,
            (a, b) => a.min(b),
        }
    }

    pub fn set_rate(&self, rate: u64) {
        self.rate.store(rate, Relaxed);
    }

    /// Accounts for `n` bytes, sleeping while this limit or one above it is
    /// overdrawn.
    pub async fn acquire(&self, n: usize) {
        self.acquire_own(n).await;
        if let Some(parent) = &self.parent {
            Box::pin(parent.acquire(n)).await;
        }
    }

    async fn acquire_own(&self, n: usize) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lower_limit_applies() {
        let global = Arc::new(Limiter::new(10_000));
        assert_eq!(Limiter::under(global.clone(), 2_000).effective_rate(), 2_000);
        assert_eq!(Limiter::under(global.clone(), 50_000).effective_rate(), 10_000);
        assert_eq!(Limiter::under(global.clone(), 0).effective_rate(), 10_000, "unlimited here: the global one");
        global.set_rate(0);
        assert_eq!(Limiter::under(global.clone(), 2_000).effective_rate(), 2_000);
        assert_eq!(Limiter::under(global, 0).effective_rate(), 0);
    }

    /// Bytes through a limiter in `secs` seconds, from `tasks` downloads at once.
    async fn pour(limiters: Vec<Arc<Limiter>>, secs: f64) -> Vec<u64> {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        let mut handles = Vec::new();
        for l in limiters {
            handles.push(tokio::spawn(async move {
                let mut got = 0u64;
                while Instant::now() < end {
                    l.acquire(4096).await;
                    got += 4096;
                }
                got
            }));
        }
        let mut out = Vec::new();
        for h in handles {
            out.push(h.await.unwrap());
        }
        out
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_keeps_to_its_own_limit_and_all_to_the_global_one() {
        // global 400 KB/s; A at 100 KB/s, B at 250 KB/s, C unlimited
        let global = Arc::new(Limiter::new(400_000));
        let a = Arc::new(Limiter::under(global.clone(), 100_000));
        let b = Arc::new(Limiter::under(global.clone(), 250_000));
        let c = Arc::new(Limiter::under(global.clone(), 0));
        let secs = 2.0;
        let got = pour(vec![a, b, c], secs).await;
        let rate = |n: u64| n as f64 / secs;
        // a bucket starts with up to half a second of credit, so allow for it
        assert!(rate(got[0]) < 100_000.0 * 1.35, "A {:.0}/s", rate(got[0]));
        assert!(rate(got[1]) < 250_000.0 * 1.35, "B {:.0}/s", rate(got[1]));
        let all: u64 = got.iter().sum();
        assert!(rate(all) < 400_000.0 * 1.35, "all {:.0}/s", rate(all));
        assert!(got.iter().all(|g| *g > 0), "each gets some: {got:?}");
    }
}
