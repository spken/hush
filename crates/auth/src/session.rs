use oauth2::TokenResponse;

use crate::{Error, login};

// Spotify's desktop client, the default librespot logs in with. Tokens from Hush's own
// client ID pass the access point but are rejected by clienttoken and login5.
const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const REDIRECT_URL: &str = "http://127.0.0.1:8898/login";
const SCOPES: [&str; 1] = ["streaming"];

/// Runs a browser login for the librespot session and returns its one-off access token.
pub async fn session_token(http: &reqwest::Client) -> Result<String, Error> {
    let client = login::client(CLIENT_ID, REDIRECT_URL)?;
    let response = login::authorize(&client, http, &SCOPES).await?;
    Ok(response.access_token().secret().clone())
}
