use crate::TrackId;

/// A track as the now-playing view shows it.
#[derive(Debug)]
pub struct Track {
    pub id: TrackId,
    pub name: Box<str>,
    pub artists: Box<[Box<str>]>,
    pub duration_ms: u32,
}

/// Something that happened on Hush's own playback device.
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
}
