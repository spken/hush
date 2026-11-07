//! Checks the gateway against a local mock of the Web API: coalescing, `Retry-After`, the
//! quota breaker, the 401 refresh and cancellation. Access tokens are real, from the stored
//! login, so a forced refresh really goes to Spotify.
//!
//! `cargo run -p webapi --example m4`

use std::{
    env, fs,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use auth::TokenProvider;
use model::{Degraded, Event};
use serde::de::IgnoredAny;
use store::Store;
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Deps, Error, Lane};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{path, path_regex},
};

const QUOTA_BODY: &str =
    r#"{"error":{"status":429,"message":"Quota exceeded","reason":"QUOTA_EXCEEDED"}}"#;

struct Shared {
    http: reqwest::Client,
    tokens: Arc<TokenProvider>,
    store: Store,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    dotenvy::dotenv().context(".env")?;
    let client_id = env::var("HUSH_CLIENT_ID").context("HUSH_CLIENT_ID is not set")?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let db = env::temp_dir().join("hush-m4.db");
    if db.exists() {
        fs::remove_file(&db)?;
    }
    let shared = Shared {
        tokens: Arc::new(TokenProvider::new(&client_id, http.clone()).await?),
        http,
        store: Store::open(&db)?,
    };

    coalescing(&shared).await?;
    tracing::info!("pass: 50 concurrent identical GETs sent 1 request");
    retry_after(&shared).await?;
    tracing::info!("pass: Retry-After: 5 held every lane for at least 5 s");
    quota(&shared).await?;
    tracing::info!("pass: QUOTA_EXCEEDED opened the breaker");
    unauthorized(&shared).await?;
    tracing::info!("pass: a 401 led to exactly one refresh");
    cancellation(&shared).await?;
    tracing::info!("pass: a cancelled queued request was never sent");
    Ok(())
}

impl Shared {
    fn gateway(&self, server: &MockServer) -> (ApiGateway, mpsc::Receiver<Event>) {
        let (events, rx) = mpsc::channel(8);
        let gateway = ApiGateway::spawn(Deps {
            http: self.http.clone(),
            tokens: self.tokens.clone(),
            store: self.store.clone(),
            base_url: server.uri().into(),
            events,
        });
        (gateway, rx)
    }
}

async fn coalescing(shared: &Shared) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(path("/same"))
        .respond_with(json(200).set_delay(Duration::from_millis(500)))
        .mount(&server)
        .await;
    let (gateway, _events) = shared.gateway(&server);
    let mut gets = JoinSet::new();
    for _ in 0..50 {
        let gateway = gateway.clone();
        gets.spawn(async move { get(&gateway, "/same", Lane::P1, CancellationToken::new()).await });
    }
    while let Some(result) = gets.join_next().await {
        result??;
    }
    let sent = received(&server, "/same").await.len();
    ensure!(sent == 1, "{sent} requests sent");
    Ok(())
}

async fn retry_after(shared: &Shared) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(path("/limited"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "5"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path_regex("^/(limited|lane)"))
        .respond_with(json(200))
        .mount(&server)
        .await;
    let (gateway, _events) = shared.gateway(&server);
    let started = Instant::now();
    let finished = |path: &'static str, lane| {
        let gateway = gateway.clone();
        tokio::spawn(async move {
            get(&gateway, path, lane, CancellationToken::new()).await?;
            anyhow::Ok(started.elapsed())
        })
    };
    let limited = finished("/limited", Lane::P1);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let urgent = finished("/lane0", Lane::P0);
    let background = finished("/lane3", Lane::P3);
    for (name, task) in [("limited", limited), ("P0", urgent), ("P3", background)] {
        let elapsed = task.await??;
        ensure!(
            elapsed >= Duration::from_secs(5),
            "{name} done after {elapsed:?}"
        );
    }
    let sent = received(&server, "/limited").await.len();
    ensure!(sent == 2, "the limited request was sent {sent} times");
    Ok(())
}

async fn quota(shared: &Shared) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(path("/quota"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "1")
                .set_body_string(QUOTA_BODY),
        )
        .mount(&server)
        .await;
    Mock::given(path("/background"))
        .respond_with(json(200))
        .mount(&server)
        .await;
    let (gateway, mut events) = shared.gateway(&server);
    let result = get(&gateway, "/quota", Lane::P1, CancellationToken::new()).await;
    ensure!(
        matches!(result, Err(Error::QuotaExceeded)),
        "quota: {result:?}"
    );
    let event = events.try_recv();
    ensure!(
        matches!(event, Ok(Event::Degraded(Degraded::QuotaExceeded))),
        "event: {event:?}"
    );
    let result = get(&gateway, "/background", Lane::P3, CancellationToken::new()).await;
    ensure!(
        matches!(result, Err(Error::QuotaExceeded)),
        "background: {result:?}"
    );
    let sent = received(&server, "/background").await.len();
    ensure!(sent == 0, "the background request was sent {sent} times");
    Ok(())
}

async fn unauthorized(shared: &Shared) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(path("/unauthorized"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let (gateway, _events) = shared.gateway(&server);
    let result = get(
        &gateway,
        "/unauthorized",
        Lane::P1,
        CancellationToken::new(),
    )
    .await;
    ensure!(
        matches!(result, Err(Error::Status(401))),
        "result: {result:?}"
    );
    let sent = received(&server, "/unauthorized").await;
    ensure!(sent.len() == 2, "sent {} times", sent.len());
    let token = |request: &Request| request.headers.get("authorization").cloned();
    ensure!(
        token(&sent[0]) != token(&sent[1]),
        "the retry did not carry a refreshed token"
    );
    Ok(())
}

async fn cancellation(shared: &Shared) -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(path_regex("^/"))
        .respond_with(json(200))
        .mount(&server)
        .await;
    let (gateway, _events) = shared.gateway(&server);
    // Spend the burst, so the next request has to queue for a token.
    for burst in ["/burst0", "/burst1", "/burst2", "/burst3"] {
        get(&gateway, burst, Lane::P1, CancellationToken::new()).await?;
    }
    let cancel = CancellationToken::new();
    let queued = {
        let (gateway, cancel) = (gateway.clone(), cancel.clone());
        tokio::spawn(async move { get(&gateway, "/cancelled", Lane::P3, cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    let result = queued.await?;
    ensure!(
        matches!(result, Err(Error::Call(kit::Error::Cancelled))),
        "result: {result:?}"
    );
    // Well past the next token, when the request would have gone out.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let sent = received(&server, "/cancelled").await.len();
    ensure!(sent == 0, "the cancelled request was sent {sent} times");
    Ok(())
}

async fn get(
    gateway: &ApiGateway,
    path: &str,
    lane: Lane,
    cancel: CancellationToken,
) -> Result<IgnoredAny, Error> {
    gateway.get(path, lane, cancel).await
}

fn json(status: u16) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_string("{}")
}

async fn received(server: &MockServer, path: &str) -> Vec<Request> {
    let mut requests = server.received_requests().await.unwrap_or_default();
    requests.retain(|request| request.url.path() == path);
    requests
}
