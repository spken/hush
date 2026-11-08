//! Album and artist pictures: fetched from Spotify's CDN, kept in a disk cache, and
//! decoded and resized off the async workers.

mod disk;
mod error;

use std::{path::PathBuf, sync::Arc};

use fast_image_resize::{PixelType, ResizeOptions, Resizer, images::Image};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use self::disk::Disk;
pub use self::error::Error;

/// Pictures fetched at once.
const FETCHES: usize = 6;
/// Pictures decoded at once.
const DECODES: usize = 2;

/// A decoded picture: RGBA rows, four bytes a pixel and not premultiplied, with the
/// average colour of its pixels.
#[derive(Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub average: [u8; 3],
}

/// Loads pictures. Cheap to clone; clones share the disk cache and the limits on
/// concurrent fetches and decodes.
#[derive(Debug, Clone)]
pub struct Images {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    http: reqwest::Client,
    disk: Disk,
    fetches: Semaphore,
    decodes: Semaphore,
}

impl Images {
    /// Pictures cached under `dir`, which is created if missing and trimmed to its size cap
    /// in the background. Must be called inside the tokio runtime.
    pub fn new(http: reqwest::Client, dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                disk: Disk::open(dir),
                fetches: Semaphore::new(FETCHES),
                decodes: Semaphore::new(DECODES),
            }),
        }
    }

    /// The picture at `url`, cropped to a square, resized to `px` by `px` and with its
    /// corners rounded to `radius` pixels, so it shows rounded on any background.
    pub async fn load(
        &self,
        url: &str,
        px: u32,
        radius: u32,
        cancel: &CancellationToken,
    ) -> Result<Picture, Error> {
        let inner = &self.inner;
        let bytes = if let Some(bytes) = inner.disk.read(url).await {
            bytes
        } else {
            let _permit = cancelled(cancel, inner.fetches.acquire()).await?;
            let bytes = cancelled(cancel, fetch(&inner.http, url)).await??;
            inner.disk.write(url, &bytes).await;
            bytes
        };
        let _permit = cancelled(cancel, inner.decodes.acquire()).await?;
        tokio::task::spawn_blocking(move || decode(&bytes, px, radius)).await?
    }
}

/// Runs `work` unless `cancel` fires first.
async fn cancelled<T>(
    cancel: &CancellationToken,
    work: impl Future<Output = T>,
) -> Result<T, Error> {
    tokio::select! {
        () = cancel.cancelled() => Err(Error::Cancelled),
        done = work => Ok(done),
    }
}

async fn fetch(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, Error> {
    let response = http.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    Ok(response.bytes().await?.into())
}

/// Decodes `bytes`, resizes them to `px` by `px`, cropping the long side, and rounds the
/// corners to `radius`.
fn decode(bytes: &[u8], px: u32, radius: u32) -> Result<Picture, Error> {
    let decoded = image::load_from_memory(bytes)?.into_rgb8();
    let (width, height) = decoded.dimensions();
    let source = Image::from_vec_u8(width, height, decoded.into_raw(), PixelType::U8x3)?;
    let mut target = Image::new(px, px, PixelType::U8x3);
    Resizer::new().resize(
        &source,
        &mut target,
        &ResizeOptions::new().fit_into_destination(Some((0.5, 0.5))),
    )?;
    let rgb = target.into_vec();
    Ok(Picture {
        width: px,
        height: px,
        average: average(&rgb),
        rgba: rounded(&rgb, px, radius),
    })
}

/// Square RGB pixels as RGBA, transparent outside corners of `radius`, with the edge
/// anti-aliased.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel coordinates and coverage fit easily in f32 and u8"
)]
fn rounded(rgb: &[u8], px: u32, radius: u32) -> Vec<u8> {
    let half = px as f32 / 2.0;
    let radius = (radius as f32).min(half);
    let inner = half - radius;
    let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
    for (index, pixel) in rgb.as_chunks::<3>().0.iter().enumerate() {
        let x = (index % px as usize) as f32 + 0.5;
        let y = (index / px as usize) as f32 + 0.5;
        // Distance outside the rounded square, negative inside it.
        let (dx, dy) = ((x - half).abs() - inner, (y - half).abs() - inner);
        let outside = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius;
        let alpha = (0.5 - outside).clamp(0.0, 1.0);
        rgba.extend_from_slice(pixel);
        rgba.push((alpha * 255.0).round() as u8);
    }
    rgba
}

/// The mean of RGB pixels.
fn average(rgb: &[u8]) -> [u8; 3] {
    let mut sums = [0u64; 3];
    for pixel in rgb.as_chunks::<3>().0 {
        for (sum, &value) in sums.iter_mut().zip(pixel) {
            *sum += u64::from(value);
        }
    }
    let pixels = (rgb.len() / 3).max(1) as u64;
    sums.map(|sum| u8::try_from(sum / pixels).unwrap_or(u8::MAX))
}
