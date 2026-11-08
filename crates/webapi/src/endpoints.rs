use std::time::Duration;

use model::{SpotifyId, web::Profile};
use tokio_util::sync::CancellationToken;

use crate::{ApiGateway, Error, Lane};

const DAY: Duration = Duration::from_hours(24);

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
