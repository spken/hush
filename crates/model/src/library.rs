/// A list the library database keeps in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    /// Liked Songs, newest first.
    LikedSongs,
    /// Saved albums, newest first.
    SavedAlbums,
    /// The user's playlists, in their Spotify order.
    Playlists,
    /// The tracks of the playlist with this key.
    Playlist(u32),
}

/// What the rows of a list show, and so which table their keys belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    Track,
    Playlist,
}

/// The text of a track row.
#[derive(Debug)]
pub struct TrackRow {
    pub key: u32,
    pub name: Box<str>,
    pub artists: Box<str>,
    pub album: Box<str>,
    pub duration_ms: u32,
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
}
