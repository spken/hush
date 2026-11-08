//! Catalog entities as a source describes them, before the database gives them keys.

use crate::SpotifyId;

/// An artist where a track or album names it.
#[derive(Debug)]
pub struct Credit {
    pub id: SpotifyId,
    pub name: Box<str>,
}

/// A track.
#[derive(Debug)]
pub struct Track {
    pub id: SpotifyId,
    pub name: Box<str>,
    pub artists: Vec<Credit>,
    pub duration_ms: u32,
}

/// What kind of release an album is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumKind {
    Album,
    /// A single or an EP.
    Single,
    Compilation,
}

/// An album; `tracks` is empty where the source only describes it.
#[derive(Debug)]
pub struct Album {
    pub id: SpotifyId,
    pub name: Box<str>,
    pub artists: Vec<Credit>,
    pub year: Option<u16>,
    pub kind: AlbumKind,
    pub tracks: Vec<Track>,
}

/// An artist with its most played tracks and its albums and singles, newest first.
#[derive(Debug)]
pub struct Artist {
    pub name: Box<str>,
    pub top_tracks: Vec<Track>,
    pub albums: Vec<Album>,
}
