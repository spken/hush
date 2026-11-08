use model::{
    AlbumKind, SpotifyId,
    catalog::{Album, Artist, Credit, Track},
    web::{self, Added, Image, Named, Page},
};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Lane};

use crate::{
    Error,
    source::{CatalogSource, Fetch},
};

/// Entries per Web API page, the most Spotify allows.
pub(crate) const PAGE: u32 = 50;
/// Entries per page of an artist's albums, the most Spotify allows dev-mode apps there.
const ARTIST_ALBUMS_PAGE: u32 = 10;

/// The catalog as the Web API serves it: the metered fallback for when spclient fails or
/// is switched off. It has no artist top tracks and reads only the user's own playlists.
pub(crate) struct WebApi {
    gateway: ApiGateway,
}

impl WebApi {
    pub(crate) fn new(gateway: ApiGateway) -> Self {
        Self { gateway }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        Ok(self
            .gateway
            .get(path, Lane::P1, CancellationToken::new())
            .await?)
    }

    /// Every entry of the list at `path` from `offset` on, `limit` per request.
    async fn rest<T: DeserializeOwned>(
        &self,
        path: &str,
        mut offset: u32,
        limit: u32,
    ) -> Result<Vec<T>, Error> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut all = Vec::new();
        loop {
            let page: Page<T> = self
                .get(&format!("{path}{separator}limit={limit}&offset={offset}"))
                .await?;
            let sent = u32::try_from(page.items.len()).unwrap_or(u32::MAX);
            all.extend(page.items);
            offset += sent;
            if sent < limit || offset >= page.total.unwrap_or(0) {
                return Ok(all);
            }
        }
    }
}

impl CatalogSource for WebApi {
    fn album(&self, id: SpotifyId) -> Fetch<'_, Album> {
        Box::pin(async move {
            let mut raw: web::Album = self.get(&format!("/albums/{id}")).await?;
            let first = raw.tracks.take().unwrap_or(Page {
                items: Vec::new(),
                total: None,
            });
            let sent = u32::try_from(first.items.len()).unwrap_or(u32::MAX);
            let mut items = first.items;
            if sent < first.total.unwrap_or(0) {
                items.extend(
                    self.rest(&format!("/albums/{id}/tracks"), sent, PAGE)
                        .await?,
                );
            }
            let mut album = album(raw).ok_or(Error::Unavailable)?;
            album.tracks = (items.into_iter())
                .filter_map(track)
                .map(|track| Track {
                    art: album.art.clone(),
                    ..track
                })
                .collect();
            Ok(album)
        })
    }

    fn artist(&self, id: SpotifyId) -> Fetch<'_, Artist> {
        Box::pin(async move {
            let raw: Named = self.get(&format!("/artists/{id}")).await?;
            let albums: Vec<web::Album> = self
                .rest(
                    &format!("/artists/{id}/albums?include_groups=album,single"),
                    0,
                    ARTIST_ALBUMS_PAGE,
                )
                .await?;
            Ok(Artist {
                art: Image::art(&raw.images),
                name: raw.name.unwrap_or_default(),
                top_tracks: Vec::new(),
                albums: albums.into_iter().filter_map(album).collect(),
            })
        })
    }

    fn playlist(&self, id: SpotifyId) -> Fetch<'_, Vec<Track>> {
        Box::pin(async move {
            let items: Vec<Added<web::Track>> = self
                .rest(&format!("/playlists/{id}/items"), 0, PAGE)
                .await?;
            Ok(items
                .into_iter()
                .filter_map(|added| track(added.item?))
                .collect())
        })
    }
}

/// A Web API track as the catalog keeps it; `None` for an episode or an entry without an
/// ID.
pub(crate) fn track(raw: web::Track) -> Option<Track> {
    if raw.kind.as_deref() == Some("episode") {
        return None;
    }
    Some(Track {
        id: raw.id?.parse().ok()?,
        name: raw.name.unwrap_or_default(),
        artists: credits(raw.artists),
        duration_ms: raw.duration_ms.unwrap_or(0),
        art: raw
            .album
            .map(|album| Image::art(&album.images))
            .unwrap_or_default(),
    })
}

/// A Web API album, with the tracks it includes, as the catalog keeps it; `None` without
/// an ID. Its tracks share its cover.
pub(crate) fn album(raw: web::Album) -> Option<Album> {
    let art = Image::art(&raw.images);
    let tracks = (raw.tracks.into_iter())
        .flat_map(|page| page.items)
        .filter_map(track)
        .map(|track| Track {
            art: art.clone(),
            ..track
        })
        .collect();
    Some(Album {
        id: raw.id?.parse().ok()?,
        name: raw.name.unwrap_or_default(),
        artists: credits(raw.artists),
        year: (raw.release_date).and_then(|date| date.get(..4).and_then(|year| year.parse().ok())),
        kind: match raw.album_type.as_deref() {
            Some("single") => AlbumKind::Single,
            Some("compilation") => AlbumKind::Compilation,
            _ => AlbumKind::Album,
        },
        art,
        tracks,
    })
}

fn credits(artists: Vec<Named>) -> Vec<Credit> {
    artists.into_iter().filter_map(credit).collect()
}

/// A Web API artist as the catalog names it; `None` without an ID.
pub(crate) fn credit(artist: Named) -> Option<Credit> {
    Some(Credit {
        id: artist.id?.parse().ok()?,
        name: artist.name.unwrap_or_default(),
        art: Image::art(&artist.images),
    })
}
