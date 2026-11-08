//! Domain types shared by every crate. Pure data, no I/O.

mod anchor;
mod art;
pub mod catalog;
mod command;
mod error;
mod event;
mod id;
mod library;
pub mod web;

pub use anchor::Anchor;
pub use art::Art;
pub use catalog::AlbumKind;
pub use command::Command;
pub use error::Error;
pub use event::{Degraded, Event, Fetch, Track};
pub use id::{SpotifyId, TrackId};
pub use library::{AlbumRow, ArtistRow, Context, Entity, Hits, List, PlaylistRow, Route, TrackRow};
