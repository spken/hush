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
}
