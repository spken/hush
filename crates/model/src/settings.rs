use serde::{Deserialize, Serialize};

/// Bands of the equalizer, by centre frequency in Hz.
pub const EQ_BANDS: usize = 10;

/// What the user chose in Settings → Playback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Playback {
    /// Whether the equalizer is on; off, the sound passes untouched.
    pub eq: bool,
    pub preamp_db: f32,
    /// One gain per band, in dB.
    pub gains_db: [f32; EQ_BANDS],
    /// Whether tracks play at a common loudness.
    pub normalisation: bool,
    /// Whether tracks follow each other without a gap.
    pub gapless: bool,
    /// The output device's ID, or `None` for the system default.
    pub output: Option<Box<str>>,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            eq: false,
            preamp_db: 0.0,
            gains_db: [0.0; EQ_BANDS],
            normalisation: true,
            gapless: true,
            output: None,
        }
    }
}

/// An audio output Settings offers.
#[derive(Debug, Clone)]
pub struct OutputDevice {
    pub id: Box<str>,
    pub name: Box<str>,
}
