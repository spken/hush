use std::pin::Pin;

use model::{
    SpotifyId,
    catalog::{Album, Artist, Track},
};

use crate::Error;

/// A catalog lookup in flight.
pub(crate) type Fetch<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

/// Where catalog metadata comes from when the database does not have it.
pub(crate) trait CatalogSource: Send + Sync {
    /// An album with all its tracks.
    fn album(&self, id: SpotifyId) -> Fetch<'_, Album>;
    /// An artist with its top tracks and its albums and singles.
    fn artist(&self, id: SpotifyId) -> Fetch<'_, Artist>;
    /// A playlist's tracks, in order; episodes and local files are left out.
    fn playlist(&self, id: SpotifyId) -> Fetch<'_, Vec<Track>>;
}
