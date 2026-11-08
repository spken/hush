use std::{fmt, hash::Hash};

use quick_cache::{Weighter, unsync::Cache};

/// What a cached value costs in memory.
pub trait Weigh {
    /// The bytes the value holds, its heap allocations included.
    fn bytes(&self) -> u64;
}

#[derive(Clone)]
struct ByBytes;

impl<K, V: Weigh> Weighter<K, V> for ByBytes {
    fn weight(&self, _: &K, value: &V) -> u64 {
        value.bytes()
    }
}

/// A cache that holds values up to a total size in bytes, evicting rarely used ones first.
pub struct ByteLru<K, V> {
    cache: Cache<K, V, ByBytes>,
}

impl<K: Eq + Hash, V: Weigh> ByteLru<K, V> {
    /// An empty cache for `capacity` bytes of values that weigh about `typical` bytes each.
    pub fn new(capacity: u64, typical: u64) -> Self {
        let items = usize::try_from(capacity / typical.max(1)).unwrap_or(usize::MAX);
        Self {
            cache: Cache::with_weighter(items, capacity, ByBytes),
        }
    }

    /// The value stored for `key`, marking it as used.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.cache.get(key)
    }

    /// Stores `value` for `key`, replacing any value it had.
    pub fn insert(&mut self, key: K, value: V) {
        self.cache.insert(key, value);
    }
}

impl<K: Eq + Hash, V: Weigh> fmt::Debug for ByteLru<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ByteLru")
            .field("bytes", &self.cache.weight())
            .finish_non_exhaustive()
    }
}
