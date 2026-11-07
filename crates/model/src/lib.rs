//! Domain types shared by every crate. Pure data, no I/O.

mod anchor;
mod error;
mod event;
mod id;

pub use anchor::Anchor;
pub use error::Error;
pub use event::{Event, Track};
pub use id::{SpotifyId, TrackId};
