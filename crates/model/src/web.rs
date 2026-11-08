//! Web API response shapes. Every field is optional: Spotify removes fields without notice.

use serde::Deserialize;

/// One page of a Web API list. Missing `Option` fields read as `None`.
#[derive(Debug, Deserialize)]
pub struct Page<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
    pub total: Option<u32>,
}

/// A list entry with when it was added: a saved track or album, or a playlist item.
#[derive(Debug, Deserialize)]
pub struct Added<T> {
    pub added_at: Option<Box<str>>,
    #[serde(alias = "track", alias = "album")]
    pub item: Option<T>,
}

/// A track, or in a playlist possibly a podcast episode.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Track {
    pub id: Option<Box<str>>,
    /// `"track"` or `"episode"`.
    #[serde(rename = "type")]
    pub kind: Option<Box<str>>,
    pub name: Option<Box<str>>,
    pub artists: Vec<Named>,
    pub album: Option<Named>,
    pub duration_ms: Option<u32>,
}

/// An album.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Album {
    pub id: Option<Box<str>>,
    pub name: Option<Box<str>>,
    pub artists: Vec<Named>,
}

/// A playlist as the user's playlist list describes it.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub id: Option<Box<str>>,
    pub name: Option<Box<str>>,
    pub owner: Option<Profile>,
    pub collaborative: Option<bool>,
    /// Changes whenever the playlist's tracks do.
    pub snapshot_id: Option<Box<str>>,
    pub items: Option<Count>,
}

/// An artist or album where another object mentions it.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Named {
    pub name: Option<Box<str>>,
}

/// How many entries a list has, where the list itself is not included.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Count {
    pub total: Option<u32>,
}

/// A Spotify user: the signed-in one from `/me`, or a playlist's owner.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: Option<Box<str>>,
    pub display_name: Option<Box<str>>,
}
