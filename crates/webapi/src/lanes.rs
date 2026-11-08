use std::{collections::VecDeque, mem, time::Duration};

use bytes::Bytes;
use tokio::time::Instant;

/// Queued requests older than this go first, so background work still progresses.
const AGING: Duration = Duration::from_mins(1);

/// How urgent a request is. A lane waits while a more urgent one has work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    /// A user action, such as playing on another device or saving to the library.
    P0,
    /// Content on screen.
    P1,
    /// Prefetch.
    P2,
    /// Background sync.
    P3,
}

/// How a request changes something, for requests that are not GETs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Put,
    Post,
}

/// A request that changes something: its method, JSON body and a number that tells it
/// apart from identical ones, which are never merged.
#[derive(Debug)]
pub(crate) struct Write {
    pub(crate) method: Method,
    pub(crate) body: Option<Box<str>>,
    pub(crate) id: u64,
}

/// What callers wait on: a path, and for a write its number; 0 for a GET, so identical
/// GETs share one request.
pub(crate) type Key = (Box<str>, u64);

/// A request that missed the cache, or a write, waiting for its turn.
#[derive(Debug)]
pub(crate) struct Job {
    pub(crate) path: Box<str>,
    pub(crate) lane: Lane,
    /// `None` for a GET.
    pub(crate) write: Option<Write>,
    /// The stale cached response and its `ETag`, to revalidate.
    pub(crate) revalidate: Option<(Box<str>, Bytes)>,
    /// Whether it already went back in line after a 429.
    pub(crate) retried: bool,
    queued_at: Instant,
}

impl Job {
    pub(crate) fn new(
        path: Box<str>,
        lane: Lane,
        write: Option<Write>,
        revalidate: Option<(Box<str>, Bytes)>,
    ) -> Self {
        Self {
            path,
            lane,
            write,
            revalidate,
            retried: false,
            queued_at: Instant::now(),
        }
    }
}

impl Job {
    /// What its callers wait on.
    pub(crate) fn key(&self) -> Key {
        (
            self.path.clone(),
            self.write.as_ref().map_or(0, |write| write.id),
        )
    }
}

/// The queue: one first-in, first-out line per lane.
#[derive(Debug, Default)]
pub(crate) struct Lanes([VecDeque<Job>; 4]);

impl Lanes {
    pub(crate) fn push_back(&mut self, job: Job) {
        self.0[job.lane as usize].push_back(job);
    }

    /// Puts `job` back at the head of its line.
    pub(crate) fn push_front(&mut self, job: Job) {
        self.0[job.lane as usize].push_front(job);
    }

    /// Takes the next job: the head of the most urgent line whose head has aged, or else of
    /// the most urgent line with any.
    pub(crate) fn pop(&mut self, now: Instant) -> Option<Job> {
        let aged = |line: &VecDeque<Job>| {
            line.front()
                .is_some_and(|job| now.saturating_duration_since(job.queued_at) >= AGING)
        };
        let line = self
            .0
            .iter()
            .position(aged)
            .or_else(|| self.0.iter().position(|line| !line.is_empty()))?;
        self.0[line].pop_front()
    }

    /// Empties the prefetch and background lines.
    pub(crate) fn take_background(&mut self) -> impl Iterator<Item = Job> {
        let [_, _, prefetch, background] = &mut self.0;
        mem::take(prefetch).into_iter().chain(mem::take(background))
    }
}
