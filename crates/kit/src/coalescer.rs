use std::{collections::HashMap, hash::Hash};

use crate::Req;

/// Callers waiting on the same piece of work, so it runs once for all of them.
#[derive(Debug)]
pub struct Coalescer<K, V> {
    waiting: HashMap<K, Vec<Req<V>>>,
}

impl<K: Eq + Hash, V: Clone> Coalescer<K, V> {
    /// Adds `req` to the callers waiting on `key`. Returns `true` when it is the first, so
    /// the work for `key` still has to be started.
    pub fn join(&mut self, key: K, req: Req<V>) -> bool {
        let waiting = self.waiting.entry(key).or_default();
        waiting.push(req);
        waiting.len() == 1
    }

    /// Forgets `key` when every caller waiting on it has cancelled, and returns whether it
    /// did, so the work can be dropped before it starts.
    pub fn drop_abandoned(&mut self, key: &K) -> bool {
        let abandoned = self
            .waiting
            .get(key)
            .is_none_or(|waiting| waiting.iter().all(Req::is_cancelled));
        if abandoned {
            self.waiting.remove(key);
        }
        abandoned
    }

    /// Answers every caller waiting on `key` with `value`.
    pub fn finish(&mut self, key: &K, value: V) {
        for req in self.waiting.remove(key).into_iter().flatten() {
            req.reply(value.clone());
        }
    }
}

impl<K, V> Default for Coalescer<K, V> {
    fn default() -> Self {
        Self {
            waiting: HashMap::new(),
        }
    }
}
