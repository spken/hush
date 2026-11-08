use std::time::Instant;

use crate::Event;

/// The last position the player reported and when; every position in between is derived
/// from it instead of polled.
#[derive(Debug, Clone, Copy)]
pub struct Anchor {
    position_ms: u32,
    at: Instant,
    is_playing: bool,
}

impl Anchor {
    /// A paused anchor at the start of a track.
    pub fn new(now: Instant) -> Self {
        Self {
            position_ms: 0,
            at: now,
            is_playing: false,
        }
    }

    /// Moves the anchor for a playback event that arrived at `now`.
    pub fn apply(&mut self, event: &Event, now: Instant) {
        let (position_ms, is_playing) = match *event {
            Event::Playing { position_ms } => (position_ms, true),
            Event::Paused { position_ms } => (position_ms, false),
            Event::Seeked { position_ms } => (position_ms, self.is_playing),
            Event::Stopped => (0, false),
            Event::TrackChanged(_)
            | Event::Volume(_)
            | Event::Active(_)
            | Event::Polled(_)
            | Event::Degraded(_)
            | Event::Syncing { .. }
            | Event::Synced
            | Event::ListChanged(_)
            | Event::Fetched { .. }
            | Event::Searched { .. } => return,
        };
        *self = Self {
            position_ms,
            at: now,
            is_playing,
        };
    }

    /// Whether the position is advancing.
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// The playback position at `now`.
    pub fn position_at(&self, now: Instant) -> u32 {
        if !self.is_playing {
            return self.position_ms;
        }
        let elapsed = now.saturating_duration_since(self.at).as_millis();
        self.position_ms
            .saturating_add(u32::try_from(elapsed).unwrap_or(u32::MAX))
    }
}
