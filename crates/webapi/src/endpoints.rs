use std::time::Duration;

use model::{
    Entity, SpotifyId,
    web::{Profile, Search},
};
use tokio_util::sync::CancellationToken;

use crate::{ApiGateway, Error, Lane};

const DAY: Duration = Duration::from_hours(24);
/// Search results per type and request, the most Spotify allows dev-mode apps.
const SEARCH_PAGE: u32 = 10;

/// How long a response stays fresh, by endpoint. Responses of other endpoints are not
/// cached: library lists sync incrementally and search has its own cache.
const TTL: [(&str, Duration); 6] = [
    ("/me", DAY),
    ("/me/following", DAY),
    ("/me/top/artists", DAY),
    ("/me/top/tracks", DAY),
    ("/me/player/recently-played", Duration::from_mins(5)),
    ("/artists/{id}/albums", DAY),
];

impl ApiGateway {
    /// The signed-in user's profile.
    pub async fn me(&self, lane: Lane, cancel: CancellationToken) -> Result<Profile, Error> {
        self.get("/me", lane, cancel).await
    }

    /// Spotify's catalog entries matching `query`: a page of each of `types`, from `offset`.
    pub async fn search(
        &self,
        query: &str,
        types: &[Entity],
        offset: u32,
        cancel: CancellationToken,
    ) -> Result<Search, Error> {
        let types: Vec<&str> = (types.iter())
            .map(|entity| match entity {
                Entity::Track => "track",
                Entity::Album => "album",
                Entity::Artist => "artist",
                Entity::Playlist => "playlist",
            })
            .collect();
        let path = format!(
            "/search?q={}&type={}&limit={SEARCH_PAGE}&offset={offset}",
            encode(query),
            types.join(",")
        );
        self.get(&path, Lane::P1, cancel).await
    }
}

/// `text` percent-encoded for a URL query.
fn encode(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
    }
    encoded
}

/// The endpoint a request path belongs to: the path without its query, with every Spotify
/// ID replaced by `{id}`.
pub(crate) fn endpoint(path: &str) -> String {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    let mut endpoint = String::with_capacity(path.len());
    for segment in path.split('/').skip(1) {
        endpoint.push('/');
        endpoint.push_str(if segment.parse::<SpotifyId>().is_ok() {
            "{id}"
        } else {
            segment
        });
    }
    endpoint
}

/// How long the response to a request for `path` stays fresh; zero when it is not cached.
pub(crate) fn ttl(path: &str) -> Duration {
    let endpoint = endpoint(path);
    TTL.iter()
        .find(|(cached, _)| *cached == endpoint)
        .map_or(Duration::ZERO, |&(_, ttl)| ttl)
}
