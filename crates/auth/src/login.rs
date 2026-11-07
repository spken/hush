use std::time::Duration;

use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, RedirectUrl, Scope, TokenUrl,
    basic::{BasicClient, BasicTokenResponse},
    url::Url,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::timeout,
};

use crate::Error;

pub(crate) type Client =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const LISTEN_ADDR: &str = "127.0.0.1:8898";
// Browsers open speculative connections that never send a request; do not wait on them.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const DONE_PAGE: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
    Connection: close\r\n\r\n<!doctype html><title>Hush</title>\
    <p style=\"font-family:system-ui\">You can close this tab and return to Hush.</p>";

/// Builds an OAuth client for a Spotify app whose redirect URI is on `LISTEN_ADDR`.
pub(crate) fn client(client_id: &str, redirect_url: &str) -> Result<Client, Error> {
    Ok(BasicClient::new(ClientId::new(client_id.to_owned()))
        .set_auth_uri(AuthUrl::new(AUTH_URL.to_owned())?)
        .set_token_uri(TokenUrl::new(TOKEN_URL.to_owned())?)
        .set_redirect_uri(RedirectUrl::new(redirect_url.to_owned())?))
}

/// Runs the PKCE browser login for `scopes` and exchanges the code for tokens.
pub(crate) async fn authorize(
    client: &Client,
    http: &reqwest::Client,
    scopes: &[&str],
) -> Result<BasicTokenResponse, Error> {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scopes(scopes.iter().map(|&scope| Scope::new(scope.to_owned())))
        .set_pkce_challenge(challenge)
        .url();
    let redirect = client.redirect_uri().map(RedirectUrl::url);

    let listener = TcpListener::bind(LISTEN_ADDR).await?;
    tracing::info!(%url, "opening the browser to log in");
    open::that_detached(url.as_str())?;

    let code = loop {
        let (stream, _) = listener.accept().await?;
        if let Some(callback) = read_callback(stream, redirect).await {
            break code(&callback, &state)?;
        }
    };
    Ok(client
        .exchange_code(code)
        .set_pkce_verifier(verifier)
        .request_async(http)
        .await?)
}

/// Reads one request and answers it if it is the redirect back from Spotify.
async fn read_callback(stream: TcpStream, redirect: Option<&Url>) -> Option<Url> {
    let redirect = redirect?;
    let mut stream = BufReader::new(stream);
    let mut request_line = String::new();
    timeout(REQUEST_TIMEOUT, stream.read_line(&mut request_line))
        .await
        .ok()?
        .ok()?;
    let target = request_line.split(' ').nth(1)?;
    let url = redirect.join(target).ok()?;
    if url.path() != redirect.path() {
        return None;
    }
    if let Err(error) = stream.get_mut().write_all(DONE_PAGE.as_bytes()).await {
        tracing::warn!(%error, "could not answer the login callback");
    }
    Some(url)
}

fn code(callback: &Url, state: &CsrfToken) -> Result<AuthorizationCode, Error> {
    let param = |name: &str| {
        callback
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if let Some(error) = param("error") {
        return Err(Error::Denied(error));
    }
    if param("state").as_deref() != Some(state.secret()) {
        return Err(Error::StateMismatch);
    }
    param("code")
        .map(AuthorizationCode::new)
        .ok_or_else(|| Error::Denied("callback has no code".into()))
}
