use std::sync::Arc;

/// Ways a catalog lookup or library sync can fail. Cheap to clone, so one failed lookup
/// can answer every caller waiting on it.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Web(#[from] webapi::Error),
    #[error("library database: {0}")]
    Store(#[source] Arc<store::Error>),
    #[error("spclient: {0}")]
    Spclient(#[source] Arc<librespot_core::Error>),
    #[error("spclient metadata: {0}")]
    Decode(#[source] Arc<protobuf::Error>),
    #[error("no catalog source could provide it")]
    Unavailable,
    #[error(transparent)]
    Call(#[from] kit::Error),
}

impl From<store::Error> for Error {
    fn from(error: store::Error) -> Self {
        Self::Store(Arc::new(error))
    }
}

impl From<librespot_core::Error> for Error {
    fn from(error: librespot_core::Error) -> Self {
        Self::Spclient(Arc::new(error))
    }
}

impl From<protobuf::Error> for Error {
    fn from(error: protobuf::Error) -> Self {
        Self::Decode(Arc::new(error))
    }
}
