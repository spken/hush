use crate::{Entity, List, PictureKey, Playback, Route};

/// Something the user asked for in the UI, handled by the Controller.
#[derive(Debug)]
pub enum Command {
    /// Pause when playing, resume when paused.
    PlayPause,
    /// Resume, or with `false` pause; nothing when already so.
    SetPlaying(bool),
    /// Skip to the next track.
    Next,
    /// Restart the track, or go to the previous one near its start.
    Previous,
    /// Jump to `position_ms` in the current track.
    Seek { position_ms: u32 },
    /// Set the device volume, from 0 to `u16::MAX`.
    SetVolume(u16),
    /// Turn shuffle on, or with `false` off.
    SetShuffle(bool),
    /// Play a list on Hush: from the track with key `start`, or from its top; shuffled or
    /// in order.
    Play {
        list: List,
        start: Option<u32>,
        shuffle: bool,
    },
    /// Show a page, remembering the one on show for `Back` unless `route` is a top-level
    /// page.
    Navigate(Route),
    /// Return to the page shown before.
    Back,
    /// Show a library list on the library page.
    OpenList(List),
    /// Rows came into view without their text; load it for these keys.
    RowsVisible { entity: Entity, keys: Vec<u32> },
    /// Sync the library with Spotify now.
    SyncLibrary,
    /// The search text changed, or with `now` the user pressed Enter: search the library at
    /// once and Spotify's catalog after a pause in typing, or at once with `now`.
    Search { query: Box<str>, now: bool },
    /// Show the next page of one kind of search result.
    SearchMore(Entity),
    /// Use these playback settings from now on and keep them.
    SetPlayback(Playback),
    /// Keep this Client ID for the Web API, or forget it when empty.
    SetClientId(Box<str>),
    /// Play the songs the search found, from the one with this key.
    PlayFound(u32),
    /// The window gained the keyboard focus.
    WindowFocused,
    /// The window was minimized, or with `true` can be seen again.
    WindowVisible(bool),
    /// List the Spotify Connect devices for the device picker.
    ListDevices,
    /// Move playback to the Spotify Connect device with this ID.
    Transfer(Box<str>),
    /// The UI needs these pictures.
    LoadPictures(Vec<PictureKey>),
}
