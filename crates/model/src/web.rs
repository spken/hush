//! Web API response shapes. Every field is optional: Spotify removes fields without notice.

use serde::Deserialize;

use crate::{Art, SpotifyId};

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
    pub duration_ms: Option<u32>,
    /// Missing where the album is the one being read.
    pub album: Option<Cover>,
}

/// An album; only `/albums/{id}` includes its first page of tracks.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Album {
    pub id: Option<Box<str>>,
    pub name: Option<Box<str>>,
    pub artists: Vec<Named>,
    /// `"album"`, `"single"` or `"compilation"`.
    pub album_type: Option<Box<str>>,
    /// `YYYY-MM-DD`, `YYYY-MM` or `YYYY`.
    pub release_date: Option<Box<str>>,
    pub images: Vec<Image>,
    pub tracks: Option<Page<Track>>,
}

/// The pictures of a track's album.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Cover {
    pub images: Vec<Image>,
}

/// One size of a picture.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Image {
    pub url: Box<str>,
    pub width: Option<u32>,
}

impl Image {
    /// The sizes of a picture as the catalog keeps them.
    pub fn art(images: &[Self]) -> Art {
        Art::new(images.iter().map(|image| (image.width, &*image.url)))
    }
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
    pub images: Option<Vec<Image>>,
}

/// An artist: where another object mentions it, or from `/artists/{id}`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Named {
    pub id: Option<Box<str>>,
    pub name: Option<Box<str>>,
    /// Only where an artist is the object read, not where another mentions it.
    pub images: Vec<Image>,
}

/// How many entries a list has, where the list itself is not included.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Count {
    pub total: Option<u32>,
}

/// `/search` results: a page for each type asked for. Spotify sends `null` for some
/// entries.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Search {
    pub tracks: Option<Page<Option<Track>>>,
    pub artists: Option<Page<Option<Named>>>,
    pub albums: Option<Page<Option<Album>>>,
    pub playlists: Option<Page<Option<Playlist>>>,
}

/// A Spotify user: the signed-in one from `/me`, or a playlist's owner.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: Option<Box<str>>,
    pub display_name: Option<Box<str>>,
}

/// A Spotify Connect device as the Web API lists it.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Device {
    pub id: Option<Box<str>>,
    pub name: Option<Box<str>>,
    /// `"Computer"`, `"Smartphone"`, `"Speaker"` and so on.
    #[serde(rename = "type")]
    pub kind: Option<Box<str>>,
    pub is_active: Option<bool>,
    pub volume_percent: Option<u32>,
}

/// The user's Spotify Connect devices.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Devices {
    pub devices: Vec<Device>,
}

/// What plays and where, from `/me/player`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Player {
    pub device: Option<Device>,
    pub progress_ms: Option<u32>,
    pub is_playing: Option<bool>,
    /// The track; an episode or an ad reads as a track without an ID.
    pub item: Option<Track>,
    pub shuffle_state: Option<bool>,
}

/// `/me/player/recently-played`: the last tracks played, newest first.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct History {
    pub items: Vec<Played>,
}

/// A track in the listening history; only where it was played from is read.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Played {
    /// ISO 8601 UTC.
    pub played_at: Option<Box<str>>,
    pub context: Option<PlayedFrom>,
}

/// The album, playlist or artist a track was played from.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PlayedFrom {
    /// Such as `spotify:playlist:{id}`, sometimes followed by `?` and session details.
    pub uri: Option<Box<str>>,
}

impl History {
    /// The playlists played from, each with when it was last played, newest first.
    pub fn playlists(self) -> Vec<(SpotifyId, Box<str>)> {
        let mut playlists: Vec<(SpotifyId, Box<str>)> = Vec::new();
        for played in self.items {
            let id = (played.context.and_then(|context| context.uri)).and_then(|uri| {
                let id = uri.strip_prefix("spotify:playlist:")?.split('?').next()?;
                id.parse().ok()
            });
            if let (Some(id), Some(at)) = (id, played.played_at)
                && !playlists.iter().any(|&(seen, _)| seen == id)
            {
                playlists.push((id, at));
            }
        }
        playlists
    }
}
