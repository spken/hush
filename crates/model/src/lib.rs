//! Domain types shared by every crate. Pure data, no I/O.

mod error;
mod id;

pub use error::Error;
pub use id::{SpotifyId, TrackId};
