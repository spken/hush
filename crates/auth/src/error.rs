use oauth2::{HttpClientError, RequestTokenError, basic::BasicErrorResponse};

pub(crate) type TokenError = RequestTokenError<HttpClientError<reqwest::Error>, BasicErrorResponse>;

/// Ways logging in or refreshing a token can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("login: {0}")]
    Io(#[from] std::io::Error),
    #[error("Spotify account URL: {0}")]
    Url(#[from] oauth2::url::ParseError),
    #[error("Spotify denied the login: {0}")]
    Denied(String),
    #[error("login callback state does not match the request")]
    StateMismatch,
    #[error("token request: {0}")]
    Token(#[source] Box<TokenError>),
    #[error("token response has no refresh token")]
    NoRefreshToken,
    #[error("Windows Credential Manager: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("blocking task: {0}")]
    Task(#[from] tokio::task::JoinError),
}

impl From<TokenError> for Error {
    fn from(error: TokenError) -> Self {
        Self::Token(Box::new(error))
    }
}
