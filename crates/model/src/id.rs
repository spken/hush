use std::{fmt, str::FromStr};

use crate::Error;

const BASE62: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const BASE62_LEN: usize = 22;

/// The 128-bit identity behind every Spotify ID, parsed from its 22-character base62 form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpotifyId(pub u128);

impl FromStr for SpotifyId {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let invalid = || Error::InvalidId(text.into());
        if text.len() != BASE62_LEN {
            return Err(invalid());
        }
        text.bytes()
            .try_fold(0u128, |id, byte| {
                let digit = BASE62.iter().position(|&d| d == byte).ok_or_else(invalid)?;
                id.checked_mul(62)
                    .and_then(|id| id.checked_add(digit as u128))
                    .ok_or_else(invalid)
            })
            .map(Self)
    }
}

impl fmt::Display for SpotifyId {
    /// The 22-character base62 form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut digits = [0u8; BASE62_LEN];
        let mut rest = self.0;
        for digit in digits.iter_mut().rev() {
            let index = u8::try_from(rest % 62).map_err(|_| fmt::Error)?;
            *digit = BASE62[usize::from(index)];
            rest /= 62;
        }
        f.write_str(std::str::from_utf8(&digits).map_err(|_| fmt::Error)?)
    }
}

/// A track, parsed from a `spotify:track:<id>` URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackId(pub SpotifyId);

impl FromStr for TrackId {
    type Err = Error;

    fn from_str(uri: &str) -> Result<Self, Error> {
        uri.strip_prefix("spotify:track:")
            .ok_or_else(|| Error::InvalidId(uri.into()))?
            .parse()
            .map(Self)
    }
}
