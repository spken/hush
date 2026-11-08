/// Ways loading a picture can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("image request: {0}")]
    Network(#[from] reqwest::Error),
    #[error("image CDN answered HTTP {0}")]
    Status(u16),
    #[error("image decode: {0}")]
    Decode(#[from] image::ImageError),
    #[error("image buffer: {0}")]
    Buffer(#[from] fast_image_resize::ImageBufferError),
    #[error("image resize: {0}")]
    Resize(#[from] fast_image_resize::ResizeError),
    #[error("image task: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("image load cancelled")]
    Cancelled,
}
