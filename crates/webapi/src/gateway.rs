use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use kit::{Coalescer, Req};
use model::{Degraded, Event};
use serde::de::DeserializeOwned;
use store::{CachedResponse, Store};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Lane, Metrics,
    bucket::Bucket,
    endpoints,
    fetch::{self, Ctx, Fetched, Outcome},
    lanes::{Job, Key, Lanes, Method, Write},
};

/// The Spotify Web API root that request paths are relative to.
pub const BASE_URL: &str = "https://api.spotify.com/v1";

const BREAKER_OPEN_FOR: Duration = Duration::from_hours(1);
const PAUSE_JITTER: Duration = Duration::from_secs(1);
const METRICS_PERIOD: Duration = Duration::from_mins(1);

/// What the gateway is built from.
#[derive(Debug)]
pub struct Deps {
    pub http: reqwest::Client,
    pub tokens: Arc<auth::TokenProvider>,
    pub store: Store,
    /// The API root, `BASE_URL` outside of examples.
    pub base_url: Box<str>,
    /// Where `Event::Degraded` and `Event::Recovered` go when the quota runs out and
    /// returns.
    pub events: mpsc::Sender<Event>,
}

/// Sends requests to the gateway actor, which stops once every handle is dropped.
#[derive(Debug, Clone)]
pub struct ApiGateway {
    tx: mpsc::Sender<Msg>,
    metrics: watch::Receiver<Metrics>,
}

enum Msg {
    Get(Req<Result<Bytes, Error>>, Box<str>, Lane),
    Send(
        Req<Result<Bytes, Error>>,
        Box<str>,
        Method,
        Option<Box<str>>,
        Lane,
    ),
}

impl ApiGateway {
    /// Starts the gateway on the current tokio runtime.
    pub fn spawn(deps: Deps) -> Self {
        let (tx, rx) = mpsc::channel(64);
        let (metrics_tx, metrics) = watch::channel(Metrics::default());
        tokio::spawn(run(deps, rx, metrics_tx));
        Self { tx, metrics }
    }

    /// The live request and cache counts.
    pub fn metrics(&self) -> watch::Receiver<Metrics> {
        self.metrics.clone()
    }

    /// GETs `path`, relative to the API root and with its query, and decodes the JSON.
    /// A fresh cached response costs no request, and identical requests in flight share one.
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        lane: Lane,
        cancel: CancellationToken,
    ) -> Result<T, Error> {
        let body = kit::call(&self.tx, cancel, |req| Msg::Get(req, path.into(), lane)).await?;
        // An empty answer, such as 204 No Content, reads as JSON `null`.
        Ok(serde_json::from_slice(if body.is_empty() {
            b"null"
        } else {
            &body
        })?)
    }

    /// Sends a request that changes something, with an optional JSON body. It is never
    /// cached or merged with another.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Box<str>>,
        lane: Lane,
        cancel: CancellationToken,
    ) -> Result<(), Error> {
        kit::call(&self.tx, cancel, |req| {
            Msg::Send(req, path.into(), method, body, lane)
        })
        .await
        .map(drop)
    }
}

/// Work the actor spawned, coming back to it.
enum Done {
    Looked {
        path: Box<str>,
        lane: Lane,
        cached: Result<Option<CachedResponse>, store::Error>,
    },
    Fetched {
        job: Job,
        fetched: Fetched,
    },
}

struct Gateway {
    ctx: Arc<Ctx>,
    lanes: Lanes,
    waiting: Coalescer<Key, Result<Bytes, Error>>,
    /// The number of the last write queued.
    writes: u64,
    bucket: Bucket,
    /// Every lane waits until then after a 429.
    paused_until: Option<Instant>,
    /// While open, after the quota ran out, the prefetch and background lanes fail.
    breaker_until: Option<Instant>,
    events: mpsc::Sender<Event>,
    metrics: watch::Sender<Metrics>,
}

async fn run(deps: Deps, mut rx: mpsc::Receiver<Msg>, metrics: watch::Sender<Metrics>) {
    let Deps {
        http,
        tokens,
        store,
        base_url,
        events,
    } = deps;
    let mut gateway = Gateway {
        ctx: Arc::new(Ctx {
            http,
            tokens,
            store,
            base_url,
        }),
        lanes: Lanes::default(),
        waiting: Coalescer::default(),
        writes: 0,
        bucket: Bucket::new(Instant::now()),
        paused_until: None,
        breaker_until: None,
        events,
        metrics,
    };
    let mut tasks = JoinSet::new();
    let mut minute = time::interval_at(Instant::now() + METRICS_PERIOD, METRICS_PERIOD);
    loop {
        let wake = gateway.dispatch(&mut tasks);
        tokio::select! {
            message = rx.recv() => match message {
                Some(Msg::Get(req, path, lane)) => gateway.get(req, path, lane, &mut tasks),
                Some(Msg::Send(req, path, method, body, lane)) => {
                    gateway.send(req, path, method, body, lane);
                }
                None => break,
            },
            Some(done) = tasks.join_next() => match done {
                Ok(done) => gateway.done(done),
                Err(error) => tracing::error!(%error, "Web API task failed"),
            },
            () = time::sleep_until(wake.unwrap_or_else(Instant::now)), if wake.is_some() => {}
            _ = minute.tick() => gateway.metrics.send_modify(Metrics::end_minute),
        }
    }
}

impl Gateway {
    fn get(
        &mut self,
        req: Req<Result<Bytes, Error>>,
        path: Box<str>,
        lane: Lane,
        tasks: &mut JoinSet<Done>,
    ) {
        if req.is_cancelled() || !self.waiting.join((path.clone(), 0), req) {
            return;
        }
        let ctx = self.ctx.clone();
        tasks.spawn(async move {
            let cached = ctx.store.cached_response(ctx.url(&path)).await;
            Done::Looked { path, lane, cached }
        });
    }

    fn send(
        &mut self,
        req: Req<Result<Bytes, Error>>,
        path: Box<str>,
        method: Method,
        body: Option<Box<str>>,
        lane: Lane,
    ) {
        if req.is_cancelled() {
            return;
        }
        self.writes += 1;
        let write = Write {
            method,
            body,
            id: self.writes,
        };
        let job = Job::new(path, lane, Some(write), None);
        self.waiting.join(job.key(), req);
        self.lanes.push_back(job);
    }

    fn done(&mut self, done: Done) {
        match done {
            Done::Looked { path, lane, cached } => self.looked(path, lane, cached),
            Done::Fetched { job, fetched } => self.fetched(job, fetched),
        }
    }

    fn looked(
        &mut self,
        path: Box<str>,
        lane: Lane,
        cached: Result<Option<CachedResponse>, store::Error>,
    ) {
        let cached = cached.unwrap_or_else(|error| {
            tracing::warn!(%error, "HTTP cache unreadable; requesting instead");
            None
        });
        match cached {
            Some(cached) if cached.fresh => {
                self.metrics.send_modify(|metrics| metrics.cache_hits += 1);
                self.waiting.finish(&(path, 0), Ok(cached.body.into()));
            }
            cached => {
                self.metrics
                    .send_modify(|metrics| metrics.cache_misses += 1);
                let revalidate = cached.and_then(|cached| Some((cached.etag?, cached.body.into())));
                self.lanes.push_back(Job::new(path, lane, None, revalidate));
            }
        }
    }

    fn fetched(&mut self, mut job: Job, Fetched { sent, outcome }: Fetched) {
        self.metrics
            .send_modify(|metrics| metrics.count(endpoints::endpoint(&job.path), sent));
        match outcome {
            Outcome::Done(result) => self.waiting.finish(&job.key(), result),
            Outcome::RateLimited(retry_after) => {
                self.pause(retry_after);
                self.metrics
                    .send_modify(|metrics| metrics.rate_limited += 1);
                if job.retried {
                    self.waiting.finish(&job.key(), Err(Error::RateLimited));
                } else {
                    job.retried = true;
                    self.lanes.push_front(job);
                }
            }
            Outcome::QuotaExceeded(retry_after) => {
                self.pause(retry_after);
                self.metrics
                    .send_modify(|metrics| metrics.quota_exceeded += 1);
                if self.breaker_until.is_none() {
                    let event = Event::Degraded(Degraded::QuotaExceeded);
                    if let Err(error) = self.events.try_send(event) {
                        tracing::warn!(%error, "quota event not delivered");
                    }
                }
                self.breaker_until = Some(Instant::now() + BREAKER_OPEN_FOR);
                self.waiting.finish(&job.key(), Err(Error::QuotaExceeded));
            }
        }
    }

    /// Stops every lane for `retry_after` plus jitter, and slows the bucket down.
    fn pause(&mut self, retry_after: Duration) {
        let now = Instant::now();
        let until = now + retry_after + kit::jitter(PAUSE_JITTER);
        self.paused_until = self.paused_until.max(Some(until));
        self.bucket.slow_down(now);
    }

    /// Sends every queued request that may go now. Returns when to try again, if work is
    /// left waiting on the bucket or a pause.
    fn dispatch(&mut self, tasks: &mut JoinSet<Done>) -> Option<Instant> {
        let now = Instant::now();
        if let Some(until) = self.breaker_until {
            if now < until {
                self.fail_background();
            } else {
                self.breaker_until = None;
                tracing::info!("Web API quota breaker closed");
                let event = Event::Recovered(Degraded::QuotaExceeded);
                if let Err(error) = self.events.try_send(event) {
                    tracing::warn!(%error, "quota event not delivered");
                }
            }
        }
        if let Some(until) = self.paused_until {
            if now < until {
                return Some(until);
            }
            self.paused_until = None;
        }
        loop {
            let job = self.lanes.pop(now)?;
            if self.waiting.drop_abandoned(&job.key()) {
                tracing::debug!(
                    path = &*job.path,
                    "cancelled request dropped before sending"
                );
                continue;
            }
            if let Some(wait) = self.bucket.take(now) {
                self.lanes.push_front(job);
                return Some(now + wait);
            }
            let ctx = self.ctx.clone();
            tasks.spawn(async move {
                let fetched = fetch::fetch(&ctx, &job).await;
                Done::Fetched { job, fetched }
            });
        }
    }

    fn fail_background(&mut self) {
        for job in self.lanes.take_background() {
            self.waiting.finish(&job.key(), Err(Error::QuotaExceeded));
        }
    }
}
