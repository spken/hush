/// Request and cache counts, for the debug overlay.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    /// HTTP requests sent, retries included.
    pub requests: u32,
    /// Lookups answered by a fresh cached response.
    pub cache_hits: u32,
    /// Lookups that needed a request.
    pub cache_misses: u32,
    /// 429 answers for the rolling rate limit.
    pub rate_limited: u32,
    /// 429 answers for an exhausted quota.
    pub quota_exceeded: u32,
    /// Requests per endpoint since the current minute began.
    pub this_minute: Vec<(Box<str>, u32)>,
}

impl Metrics {
    /// Counts `sent` requests to `endpoint`.
    pub(crate) fn count(&mut self, endpoint: String, sent: u32) {
        if sent == 0 {
            return;
        }
        self.requests += sent;
        match self
            .this_minute
            .iter_mut()
            .find(|(counted, _)| **counted == *endpoint)
        {
            Some((_, requests)) => *requests += sent,
            None => self.this_minute.push((endpoint.into(), sent)),
        }
    }

    /// Logs the minute's requests per endpoint and starts a new minute.
    pub(crate) fn end_minute(&mut self) {
        for (endpoint, requests) in self.this_minute.drain(..) {
            tracing::info!(
                endpoint = &*endpoint,
                requests,
                "Web API requests last minute"
            );
        }
    }
}
