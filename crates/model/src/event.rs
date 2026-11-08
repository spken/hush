use crate::{Art, Entity, Hits, List, TrackId};

/// A track as the now-playing view shows it.
#[derive(Debug)]
pub struct Track {
    pub id: TrackId,
    pub name: Box<str>,
    pub artists: Box<[Box<str>]>,
    pub duration_ms: u32,
    pub art: Art,
}

/// Why Hush is running with less than full service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degraded {
    /// The Web API quota ran out; only cached data and urgent requests are served.
    QuotaExceeded,
}

/// How a catalog lookup ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    /// The database already had the list.
    Unchanged,
    /// The list was fetched and stored.
    Changed,
    /// No catalog source could provide it.
    Failed,
}

/// Something that happened on Hush's own playback device or to its services.
#[derive(Debug)]
pub enum Event {
    /// A track was loaded; a `Playing` or `Paused` with its start position follows.
    TrackChanged(Track),
    /// Playback started or resumed at `position_ms`.
    Playing { position_ms: u32 },
    /// Playback paused at `position_ms`.
    Paused { position_ms: u32 },
    /// The position jumped (a seek or a drift correction) without changing play state.
    Seeked { position_ms: u32 },
    /// Playback stopped and nothing is loaded.
    Stopped,
    /// The device volume changed, from 0 to `u16::MAX`.
    Volume(u16),
    /// Hush became, or stopped being, the active Spotify Connect device.
    Active(bool),
    /// A service fell back to a degraded mode.
    Degraded(Degraded),
    /// Library sync is running: `done` of `total` entries of the list being read, both 0
    /// while unknown.
    Syncing { done: u32, total: u32 },
    /// Library sync ended.
    Synced,
    /// A list changed in the database.
    ListChanged(List),
    /// A catalog lookup of a list ended.
    Fetched { list: List, outcome: Fetch },
    /// A search of Spotify's catalog for `query` ended: its first page, or with `more` a
    /// further page of one kind; `None` when it failed.
    Searched {
        query: Box<str>,
        more: bool,
        found: Option<Vec<(Entity, Hits)>>,
    },
}
