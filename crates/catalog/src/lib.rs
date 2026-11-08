//! The music catalog: library sync with the Web API, and albums, artists and playlists
//! looked up in the database, then spclient, then the Web API.

mod error;
mod service;
mod source;
mod spclient;
mod sync;
mod web_api;

pub use error::Error;
pub use service::{CatalogService, Deps};
pub use sync::{Library, sync};
