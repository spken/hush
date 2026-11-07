//! The Web API gateway: the only code that talks to `api.spotify.com`.

mod bucket;
mod endpoints;
mod error;
mod fetch;
mod gateway;
mod lanes;
mod metrics;

pub use error::Error;
pub use gateway::{ApiGateway, BASE_URL, Deps};
pub use lanes::Lane;
pub use metrics::Metrics;
