use std::fmt;

/// Errors any crate can report to the Controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Text that is not a valid Spotify ID or URI.
    InvalidId(Box<str>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(text) => write!(f, "not a valid Spotify ID or URI: {text}"),
        }
    }
}

impl std::error::Error for Error {}
