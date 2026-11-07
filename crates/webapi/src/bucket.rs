use std::time::Duration;

use tokio::time::Instant;

// Conservative guesses: Spotify publishes no thresholds, only a rolling 30 s window.
const RATE: u32 = 20;
const WINDOW: Duration = Duration::from_secs(30);
const BURST: f64 = 4.0;
const SLOWED_FOR: Duration = Duration::from_mins(10);
const RECOVER_EVERY: Duration = Duration::from_mins(5);

/// A token bucket of `RATE` requests per `WINDOW`, halved after a 429 and recovering one
/// step per clean `RECOVER_EVERY` once `SLOWED_FOR` has passed.
#[derive(Debug)]
pub(crate) struct Bucket {
    rate: u32,
    tokens: f64,
    refilled: Instant,
    recover_at: Option<Instant>,
}

impl Bucket {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            rate: RATE,
            tokens: BURST,
            refilled: now,
            recover_at: None,
        }
    }

    /// Takes a token, or returns how long until one is available.
    pub(crate) fn take(&mut self, now: Instant) -> Option<Duration> {
        if let Some(at) = self.recover_at
            && now >= at
        {
            self.rate += 1;
            self.recover_at = (self.rate < RATE).then_some(now + RECOVER_EVERY);
        }
        let interval = WINDOW / self.rate;
        let earned =
            now.saturating_duration_since(self.refilled).as_secs_f64() / interval.as_secs_f64();
        self.tokens = (self.tokens + earned).min(BURST);
        self.refilled = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return None;
        }
        Some(interval.mul_f64(1.0 - self.tokens))
    }

    /// Halves the rate after a 429.
    pub(crate) fn slow_down(&mut self, now: Instant) {
        self.rate = (self.rate / 2).max(1);
        self.recover_at = Some(now + SLOWED_FOR);
    }
}
