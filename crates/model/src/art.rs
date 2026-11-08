/// A picture at the sizes Spotify serves it, as `width url` lines, narrowest first. Empty
/// when Spotify has none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Art(Box<str>);

/// One decoded picture as the UI asks for it: its URL, its size in pixels and its corner
/// radius.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PictureKey {
    pub url: Box<str>,
    pub px: u32,
    pub radius: u32,
}

/// The width assumed for a size Spotify does not give one, as for most playlist pictures.
const UNKNOWN_WIDTH: u32 = 640;

impl Art {
    /// The picture at these `(width, url)` sizes.
    pub fn new<'a>(sizes: impl IntoIterator<Item = (Option<u32>, &'a str)>) -> Self {
        let mut sizes: Vec<(u32, &str)> = (sizes.into_iter())
            .filter(|(_, url)| !url.is_empty())
            .map(|(width, url)| (width.unwrap_or(UNKNOWN_WIDTH), url))
            .collect();
        sizes.sort_unstable_by_key(|(width, _)| *width);
        let lines: Vec<String> = (sizes.iter())
            .map(|(width, url)| format!("{width} {url}"))
            .collect();
        Self(lines.join("\n").into())
    }

    /// A picture as `as_str` wrote it.
    pub fn from_stored(text: String) -> Self {
        Self(text.into())
    }

    /// The text that `from_stored` reads back.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The URL of the narrowest size at least `px` wide, or else of the widest, in a
    /// picture's stored text; `None` when it has no sizes.
    pub fn url(text: &str, px: u32) -> Option<&str> {
        let mut widest = None;
        for line in text.lines() {
            let (width, url) = line.split_once(' ')?;
            widest = Some(url);
            if width.parse::<u32>().is_ok_and(|width| width >= px) {
                break;
            }
        }
        widest
    }
}
