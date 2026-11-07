use std::sync::Arc;

/// Ways a Web API request can fail. Cheap to clone, so one failed request can answer every
/// caller waiting on it.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("Web API request: {0}")]
    Network(#[source] Arc<reqwest::Error>),
    #[error("Web API answered HTTP {0}")]
    Status(u16),
    #[error("Web API rate limit hit")]
    RateLimited,
    #[error("Web API quota exceeded")]
    QuotaExceeded,
    #[error("Web API access token: {0}")]
    Auth(#[source] Arc<auth::Error>),
    #[error("Web API response: {0}")]
    Decode(#[source] Arc<serde_json::Error>),
    #[error(transparent)]
    Call(#[from] kit::Error),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(Arc::new(error))
    }
}

impl From<auth::Error> for Error {
    fn from(error: auth::Error) -> Self {
        Self::Auth(Arc::new(error))
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(Arc::new(error))
    }
}
