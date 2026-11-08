//! Domain types shared by every crate. Pure data, no I/O.

mod anchor;
pub mod catalog;
mod command;
mod error;
mod event;
mod id;
mod library;
pub mod web;

pub use anchor::Anchor;
pub use catalog::AlbumKind;
pub use command::Command;
pub use error::Error;
pub use event::{Degraded, Event, Fetch, Track};
pub use id::{SpotifyId, TrackId};
pub use library::{AlbumRow, Context, Entity, List, PlaylistRow, Route, TrackRow};
