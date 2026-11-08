//! Pictures for the UI: decoded ones in a memory-capped cache on the UI thread, missing
//! ones asked of the Controller.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use kit::{ByteLru, Weigh};
use model::{Art as Sizes, Command};
use slint::{Color, ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use ui::{Art, MainWindow};

use super::{View, int};
use crate::controller;

/// Decoded pictures kept in memory.
const CACHE_BYTES: u64 = 16 * 1024 * 1024;
/// A tile's picture, the most common size.
const TYPICAL_BYTES: u64 = 320 * 320 * 4;

/// A picture as the UI asks for it: its URL, its size in pixels and its corner radius.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PictureKey {
    pub(crate) url: Box<str>,
    pub(crate) px: u32,
    pub(crate) radius: u32,
}

struct Decoded(Image);

impl Weigh for Decoded {
    fn bytes(&self) -> u64 {
        let size = self.0.size();
        u64::from(size.width) * u64::from(size.height) * 4
    }
}

struct Cache {
    decoded: ByteLru<PictureKey, Decoded>,
    /// Asked of the Controller and not arrived yet.
    pending: HashSet<PictureKey>,
}

impl View {
    /// Hands the UI a picture it asked for: decoded, or `None` when loading failed; `retry`
    /// says whether asking again may succeed.
    pub(crate) fn picture(&self, key: PictureKey, picture: Option<images::Picture>, retry: bool) {
        self.update(move |window| {
            let image = picture.as_ref().map_or_else(Image::default, image);
            window.global::<Art>().invoke_arrived(
                key.url.as_ref().into(),
                int(key.px),
                int(key.radius),
                image,
                retry,
            );
        });
    }
}

impl View {
    /// Floods the canvas with a decoded cover, or clears it for `None`, cross-fading from
    /// what it showed.
    pub(crate) fn backdrop(&self, picture: Option<images::Picture>) {
        self.update(move |window| {
            let (image, colour) = picture.map_or_else(
                || (Image::default(), Color::from_argb_u8(0, 0, 0, 0)),
                |picture| {
                    let [red, green, blue] = picture.average;
                    (image(&picture), Color::from_rgb_u8(red, green, blue))
                },
            );
            let art = window.global::<Art>();
            let to_b = !art.get_backdrop_b_front();
            if to_b {
                art.set_backdrop_b(image);
                art.set_backdrop_b_colour(colour);
            } else {
                art.set_backdrop_a(image);
                art.set_backdrop_a_colour(colour);
            }
            art.set_backdrop_b_front(to_b);
        });
    }
}

/// A decoded picture as Slint shows it.
fn image(picture: &images::Picture) -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &picture.rgba,
        picture.width,
        picture.height,
    ))
}

/// Answers the UI's requests for pictures from the cache, and asks the Controller for the
/// ones it lacks.
pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let cache = Rc::new(RefCell::new(Cache {
        decoded: ByteLru::new(CACHE_BYTES, TYPICAL_BYTES),
        pending: HashSet::new(),
    }));
    let art = window.global::<Art>();
    let lookup = Rc::clone(&cache);
    let controller = controller.clone();
    art.on_image(move |sizes, px, radius, _revision| {
        let (Ok(px), Ok(radius)) = (u32::try_from(px), u32::try_from(radius)) else {
            return Image::default();
        };
        let Some(url) = Sizes::url(&sizes, px) else {
            return Image::default();
        };
        let key = PictureKey {
            url: url.into(),
            px,
            radius,
        };
        let mut cache = lookup.borrow_mut();
        if let Some(decoded) = cache.decoded.get(&key) {
            return decoded.0.clone();
        }
        if cache.pending.insert(key.clone()) {
            controller.send(Command::LoadPicture {
                url: key.url,
                px,
                radius,
            });
        }
        Image::default()
    });
    let weak = window.as_weak();
    art.on_arrived(move |url, px, radius, image, retry| {
        let (Ok(px), Ok(radius)) = (u32::try_from(px), u32::try_from(radius)) else {
            return;
        };
        let key = PictureKey {
            url: url.as_str().into(),
            px,
            radius,
        };
        let mut cache = cache.borrow_mut();
        cache.pending.remove(&key);
        let loaded = image.size().width > 0;
        if loaded {
            cache.decoded.insert(key, Decoded(image));
        }
        if (loaded || retry)
            && let Some(window) = weak.upgrade()
        {
            let art = window.global::<Art>();
            art.set_revision(art.get_revision().wrapping_add(1));
        }
    });
}
