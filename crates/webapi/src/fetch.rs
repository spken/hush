use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use reqwest::{
    Response, StatusCode,
    header::{ETAG, IF_NONE_MATCH, RETRY_AFTER},
};
use store::Store;

use crate::{Error, Lane, endpoints, lanes::Job};

/// Tries per request for server and network errors, on the urgent lanes only.
const ATTEMPTS: u32 = 3;
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(5);
const QUOTA_EXCEEDED: &[u8] = b"QUOTA_EXCEEDED";

/// What every request is sent with.
#[derive(Debug)]
pub(crate) struct Ctx {
    pub(crate) http: reqwest::Client,
    pub(crate) tokens: Arc<auth::TokenProvider>,
    pub(crate) store: Store,
    pub(crate) base_url: Box<str>,
}

impl Ctx {
    /// The full URL of a request path, which is also its cache key.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }
}

/// The result of sending a request.
#[derive(Debug)]
pub(crate) struct Fetched {
    /// HTTP requests it took, retries included.
    pub(crate) sent: u32,
    pub(crate) outcome: Outcome,
}

#[derive(Debug)]
pub(crate) enum Outcome {
    Done(Result<Bytes, Error>),
    /// A 429 for the rolling rate limit, to be retried after the delay.
    RateLimited(Duration),
    /// A 429 because the quota ran out; urgent requests may resume after the delay.
    QuotaExceeded(Duration),
}

/// Sends `job`, refreshing a rejected token once and retrying server and network errors
/// on the urgent lanes. Successful responses are cached when their endpoint has a TTL.
pub(crate) async fn fetch(ctx: &Ctx, job: &Job) -> Fetched {
    let mut sent = 0;
    let outcome = send(ctx, job, &mut sent).await;
    Fetched { sent, outcome }
}

async fn send(ctx: &Ctx, job: &Job, sent: &mut u32) -> Outcome {
    let url = ctx.url(&job.path);
    let mut token = match ctx.tokens.access_token().await {
        Ok(token) => token,
        Err(error) => return Outcome::Done(Err(error.into())),
    };
    let mut refreshed = false;
    let mut failures = 0;
    loop {
        *sent += 1;
        let mut request = ctx.http.get(&url).bearer_auth(&token);
        if let Some((etag, _)) = &job.revalidate {
            request = request.header(IF_NONE_MATCH, &**etag);
        }
        let failure = match request.send().await {
            Err(error) => Error::from(error),
            Ok(response) => {
                let status = response.status();
                tracing::debug!(endpoint = endpoints::endpoint(&job.path), %status, "Web API request done");
                match status {
                    StatusCode::UNAUTHORIZED if !refreshed => {
                        refreshed = true;
                        match ctx.tokens.refresh_rejected(&token).await {
                            Ok(fresh) => token = fresh,
                            Err(error) => return Outcome::Done(Err(error.into())),
                        }
                        continue;
                    }
                    StatusCode::TOO_MANY_REQUESTS => return limited(&url, response).await,
                    StatusCode::NOT_MODIFIED => {
                        let Some((etag, body)) = &job.revalidate else {
                            return Outcome::Done(Err(Error::Status(status.as_u16())));
                        };
                        let body = keep(ctx, url, &job.path, Some(etag.clone()), body.clone());
                        return Outcome::Done(Ok(body.await));
                    }
                    status if status.is_success() => {
                        return Outcome::Done(ok(ctx, url, &job.path, response).await);
                    }
                    status if status.is_server_error() => Error::Status(status.as_u16()),
                    status => return Outcome::Done(Err(Error::Status(status.as_u16()))),
                }
            }
        };
        failures += 1;
        if job.lane > Lane::P1 || failures == ATTEMPTS {
            return Outcome::Done(Err(failure));
        }
        tracing::debug!(%failure, failures, "Web API request failed, retrying");
        tokio::time::sleep(kit::backoff(failures - 1)).await;
    }
}

async fn ok(ctx: &Ctx, url: String, path: &str, response: Response) -> Result<Bytes, Error> {
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|etag| etag.to_str().ok())
        .map(Into::into);
    let body = response.bytes().await?;
    Ok(keep(ctx, url, path, etag, body).await)
}

/// Caches a response when its endpoint has a TTL, and returns its body.
async fn keep(ctx: &Ctx, url: String, path: &str, etag: Option<Box<str>>, body: Bytes) -> Bytes {
    let ttl = endpoints::ttl(path);
    if !ttl.is_zero() {
        tracing::debug!(
            endpoint = endpoints::endpoint(path),
            etag = etag.is_some(),
            "caching Web API response"
        );
        if let Err(error) = ctx.store.store_response(url, etag, body.clone(), ttl).await {
            tracing::warn!(%error, "could not cache a Web API response");
        }
    }
    body
}

async fn limited(url: &str, response: Response) -> Outcome {
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok()?.trim().parse().ok())
        .map_or(DEFAULT_RETRY_AFTER, Duration::from_secs);
    let body = response.bytes().await.unwrap_or_else(|error| {
        tracing::debug!(%error, "429 body unreadable; treating it as a rate limit");
        Bytes::new()
    });
    // Spotify does not document where the reason sits in the body, so search all of it.
    if body
        .windows(QUOTA_EXCEEDED.len())
        .any(|window| window == QUOTA_EXCEEDED)
    {
        tracing::warn!(
            url,
            ?retry_after,
            body = %String::from_utf8_lossy(&body),
            "Web API quota exceeded"
        );
        Outcome::QuotaExceeded(retry_after)
    } else {
        tracing::warn!(url, ?retry_after, "Web API rate limit hit");
        Outcome::RateLimited(retry_after)
    }
}
