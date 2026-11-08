use crate::{AlbumKind, Art, SpotifyId};

/// A list the database keeps in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum List {
    /// Liked Songs, newest first.
    LikedSongs,
    /// Saved albums, newest first.
    SavedAlbums,
    /// The user's playlists, in their Spotify order.
    Playlists,
    /// The tracks of the playlist with this key.
    Playlist(u32),
    /// The tracks of the album with this key.
    Album(u32),
    /// The most played tracks of the artist with this key.
    ArtistTop(u32),
    /// The albums and singles of the artist with this key, newest first.
    ArtistAlbums(u32),
}

/// What a key names, and so which table it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    Track,
    Album,
    Artist,
    Playlist,
}

/// A page of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    NowPlaying,
    Library,
    /// The album with this key.
    Album(u32),
    /// The artist with this key.
    Artist(u32),
    Search,
}

/// What Spotify Connect plays from, so next and previous follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    LikedSongs,
    Album(SpotifyId),
    Artist(SpotifyId),
    Playlist(SpotifyId),
    /// These tracks, in order.
    Tracks(Box<[SpotifyId]>),
}

/// One kind of search result from Spotify's catalog: the keys found so far, in Spotify's
/// order.
#[derive(Debug, Clone, Default)]
pub struct Hits {
    pub keys: Vec<u32>,
    /// The offset the next page starts at.
    pub next: u32,
    /// How many results Spotify has in all.
    pub total: u32,
}

/// The text of a track row.
#[derive(Debug)]
pub struct TrackRow {
    pub key: u32,
    pub name: Box<str>,
    pub artists: Box<str>,
    /// The first artist, where the track's source named its ID.
    pub artist_key: Option<u32>,
    pub duration_ms: u32,
    pub art: Art,
}

/// The text of an artist row.
#[derive(Debug)]
pub struct ArtistRow {
    pub key: u32,
    pub name: Box<str>,
    pub art: Art,
}

/// The text of an album row.
#[derive(Debug)]
pub struct AlbumRow {
    pub key: u32,
    pub name: Box<str>,
    pub artists: Box<str>,
    /// The first artist, where the album's source named its ID.
    pub artist_key: Option<u32>,
    pub year: Option<u16>,
    pub kind: AlbumKind,
    pub art: Art,
}

/// The text of a playlist row.
#[derive(Debug)]
pub struct PlaylistRow {
    pub key: u32,
    pub name: Box<str>,
    pub owner: Box<str>,
    /// How many tracks Spotify says it has.
    pub tracks: u32,
    /// Whether its tracks are in the database.
    pub synced: bool,
    pub art: Art,
}
