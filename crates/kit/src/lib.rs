//! Building blocks the service actors share: request/reply with cancellation, request
//! coalescing, retry timing and a memory-bounded cache.

mod byte_lru;
mod coalescer;
mod req;
mod retry;

pub use byte_lru::{ByteLru, Weigh};
pub use coalescer::Coalescer;
pub use req::{Error, Req, call};
pub use retry::{backoff, jitter};
