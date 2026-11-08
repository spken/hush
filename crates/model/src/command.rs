use crate::{Entity, List, Route};

/// Something the user asked for in the UI, handled by the Controller.
#[derive(Debug)]
pub enum Command {
    /// Pause when playing, resume when paused.
    PlayPause,
    /// Skip to the next track.
    Next,
    /// Restart the track, or go to the previous one near its start.
    Previous,
    /// Jump to `position_ms` in the current track.
    Seek { position_ms: u32 },
    /// Set the device volume, from 0 to `u16::MAX`.
    SetVolume(u16),
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
}
