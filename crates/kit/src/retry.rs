use std::{
    hash::{BuildHasher, Hasher, RandomState},
    time::Duration,
};

/// The delay before retry `attempt`, counted from 0: 1, 2, 4 s… plus up to half as much
/// again of jitter.
pub fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_secs(1 << attempt.min(6));
    base + jitter(base / 2)
}

/// A random duration from zero to `max`, so retries from many callers do not line up.
pub fn jitter(max: Duration) -> Duration {
    // Every `RandomState` is freshly keyed, so hashing nothing yields a random number.
    let random = RandomState::new().build_hasher().finish();
    let max_nanos = u64::try_from(max.as_nanos()).unwrap_or(u64::MAX);
    Duration::from_nanos(random % max_nanos.saturating_add(1))
}
