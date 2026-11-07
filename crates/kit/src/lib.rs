//! Building blocks the service actors share: request/reply with cancellation, request
//! coalescing and retry timing.

mod coalescer;
mod req;
mod retry;

pub use coalescer::Coalescer;
pub use req::{Error, Req, call};
pub use retry::{backoff, jitter};
