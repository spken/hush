use std::time::{Duration, Instant};

use keyring::Entry;
use oauth2::{
    AccessToken, RefreshToken, RequestTokenError, TokenResponse, basic::BasicTokenResponse,
};
use tokio::sync::Mutex;

use crate::{
    Error,
    error::TokenError,
    login::{self, Client},
};

const REDIRECT_URL: &str = "http://127.0.0.1:8898/callback";
const SCOPES: [&str; 9] = [
    "user-library-read",
    "user-library-modify",
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-private",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-recently-played",
    "user-top-read",
];
const KEYRING_SERVICE: &str = "Hush";
const KEYRING_USER: &str = "spotify-refresh-token";
const DEFAULT_LIFETIME: Duration = Duration::from_secs(3600);

/// Hands out valid Web API access tokens, refreshing them before they expire.
#[derive(Debug)]
pub struct TokenProvider {
    client: Client,
    http: reqwest::Client,
    // Held across the refresh request, so only one refresh is ever in flight.
    token: Mutex<Token>,
}

#[derive(Debug)]
struct Token {
    access: AccessToken,
    refresh: RefreshToken,
    refresh_at: Instant,
}

impl TokenProvider {
    /// Signs in with the stored refresh token, or through the browser when there is none
    /// or Spotify rejects it.
    pub async fn new(client_id: &str, http: reqwest::Client) -> Result<Self, Error> {
        let client = login::client(client_id, REDIRECT_URL)?;

        let stored = keyring(|entry| match entry.get_password() {
            Err(keyring::Error::NoEntry) => Ok(None),
            password => password.map(|secret| Some(RefreshToken::new(secret))),
        })
        .await?;
        let restored = match stored {
            Some(refresh) => match refresh_request(&client, &http, &refresh).await {
                Err(RequestTokenError::ServerResponse(error)) => {
                    tracing::warn!(%error, "stored refresh token rejected, logging in again");
                    None
                }
                response => Some(accept(response?, Some(refresh)).await?),
            },
            None => None,
        };
        let token = if let Some(token) = restored {
            token
        } else {
            let response = login::authorize(&client, &http, &SCOPES).await?;
            accept(response, None).await?
        };

        Ok(Self {
            client,
            http,
            token: Mutex::new(token),
        })
    }

    /// Returns a valid access token, refreshing it once 80% of its lifetime has passed.
    pub async fn access_token(&self) -> Result<String, Error> {
        let mut token = self.token.lock().await;
        if Instant::now() >= token.refresh_at {
            self.refresh(&mut token).await?;
        }
        Ok(token.access.secret().clone())
    }

    /// Returns a new access token after the Web API rejected `rejected`. Refreshes only if
    /// no other caller has replaced that token already.
    pub async fn refresh_rejected(&self, rejected: &str) -> Result<String, Error> {
        let mut token = self.token.lock().await;
        if token.access.secret() == rejected {
            self.refresh(&mut token).await?;
        }
        Ok(token.access.secret().clone())
    }

    async fn refresh(&self, token: &mut Token) -> Result<(), Error> {
        let response = refresh_request(&self.client, &self.http, &token.refresh).await?;
        *token = accept(response, Some(token.refresh.clone())).await?;
        tracing::info!("access token refreshed");
        Ok(())
    }
}

async fn refresh_request(
    client: &Client,
    http: &reqwest::Client,
    refresh: &RefreshToken,
) -> Result<BasicTokenResponse, TokenError> {
    client
        .exchange_refresh_token(refresh)
        .request_async(http)
        .await
}

/// Turns a token response into a `Token`, storing the rotated refresh token when Spotify
/// sends one.
async fn accept(
    response: BasicTokenResponse,
    previous: Option<RefreshToken>,
) -> Result<Token, Error> {
    let refresh = match response.refresh_token() {
        Some(rotated) => {
            let secret = rotated.secret().clone();
            keyring(move |entry| entry.set_password(&secret)).await?;
            rotated.clone()
        }
        None => previous.ok_or(Error::NoRefreshToken)?,
    };
    let lifetime = response.expires_in().unwrap_or(DEFAULT_LIFETIME);
    Ok(Token {
        access: response.access_token().clone(),
        refresh,
        refresh_at: Instant::now() + lifetime * 4 / 5,
    })
}

async fn keyring<T: Send + 'static>(
    op: impl FnOnce(&Entry) -> keyring::Result<T> + Send + 'static,
) -> Result<T, Error> {
    let entry = || Entry::new(KEYRING_SERVICE, KEYRING_USER).and_then(|entry| op(&entry));
    Ok(tokio::task::spawn_blocking(entry).await??)
}
