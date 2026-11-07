//! Domain types shared by every crate. Pure data, no I/O.

mod anchor;
mod command;
mod error;
mod event;
mod id;
mod profile;

pub use anchor::Anchor;
pub use command::Command;
pub use error::Error;
pub use event::{Degraded, Event, Track};
pub use id::{SpotifyId, TrackId};
pub use profile::Profile;
