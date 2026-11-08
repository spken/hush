//! The music catalog: the library kept in the database in step with Spotify.

mod sync;

pub use sync::{Error, Library, sync};
