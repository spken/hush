use std::collections::HashMap;

use http::Method;
use librespot_core::{Session, SpotifyUri, spotify_id::SpotifyId as LibrespotId};
use librespot_protocol::{
    extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery},
    extension_kind::ExtensionKind,
    metadata::{self, album::Type, image::Size},
    playlist4_external::SelectedListContent,
};
use model::{
    AlbumKind, Art, SpotifyId,
    catalog::{Album, Artist, Credit, PlaylistItem, Track},
};
use protobuf::{EnumOrUnknown, Message};

use crate::{
    Error,
    source::{CatalogSource, Fetch},
};

/// Entities per `extended-metadata` request. 300 tracks answered in one request in testing;
/// the limit is unknown.
const BATCH: usize = 250;
/// Playlists per page of the user's playlist list.
const ROOTLIST_PAGE: usize = 500;
/// Where Spotify serves pictures, by file ID.
const IMAGE_CDN: &str = "https://i.scdn.co/image/";
const HEX: &[u8; 16] = b"0123456789abcdef";

/// The catalog as librespot's session reads it from spclient, which costs no Web API
/// requests. Lists of entities come in one batched request.
pub(crate) struct Spclient {
    session: Session,
}

impl Spclient {
    pub(crate) fn new(session: Session) -> Self {
        Self { session }
    }

    /// Fetches entities of one kind in batches and returns them in the order of `uris`;
    /// entities spclient does not answer for are left out.
    async fn batch<M: Message>(
        &self,
        uris: &[String],
        kind: ExtensionKind,
    ) -> Result<Vec<M>, Error> {
        let mut found: HashMap<String, M> = HashMap::with_capacity(uris.len());
        for chunk in uris.chunks(BATCH) {
            let request = BatchedEntityRequest {
                entity_request: (chunk.iter())
                    .map(|uri| EntityRequest {
                        entity_uri: uri.clone(),
                        query: vec![ExtensionQuery {
                            extension_kind: EnumOrUnknown::new(kind),
                            ..ExtensionQuery::default()
                        }],
                        ..EntityRequest::default()
                    })
                    .collect(),
                ..BatchedEntityRequest::default()
            };
            let response = self
                .session
                .spclient()
                .get_extended_metadata(request)
                .await?;
            for data in
                (response.extended_metadata.into_iter()).flat_map(|array| array.extension_data)
            {
                if let Some(any) = data.extension_data.into_option() {
                    found.insert(data.entity_uri, M::parse_from_bytes(&any.value)?);
                }
            }
        }
        Ok(uris.iter().filter_map(|uri| found.remove(uri)).collect())
    }

    async fn tracks(&self, uris: &[String]) -> Result<Vec<Track>, Error> {
        let tracks = self
            .batch::<metadata::Track>(uris, ExtensionKind::TRACK_V4)
            .await?;
        Ok(tracks.iter().filter_map(track).collect())
    }
}

impl CatalogSource for Spclient {
    fn album(&self, id: SpotifyId) -> Fetch<'_, Album> {
        Box::pin(async move {
            let raw = (self.session.spclient())
                .get_album_metadata(&SpotifyUri::Album { id: librespot(id) })
                .await?;
            let raw = metadata::Album::parse_from_bytes(&raw)?;
            let tracks: Vec<String> = (raw.disc.iter())
                .flat_map(|disc| &disc.track)
                .filter_map(|track| uri("track", track.gid()))
                .collect();
            let mut album = album(&raw).ok_or(Error::Unavailable)?;
            album.tracks = self.tracks(&tracks).await?;
            Ok(album)
        })
    }

    fn artist(&self, id: SpotifyId) -> Fetch<'_, Artist> {
        Box::pin(async move {
            let raw = (self.session.spclient())
                .get_artist_metadata(&SpotifyUri::Artist { id: librespot(id) })
                .await?;
            let raw = metadata::Artist::parse_from_bytes(&raw)?;
            let country = self.session.country();
            let top = (raw.top_track.iter())
                .find(|top| top.country() == country)
                .or_else(|| raw.top_track.iter().find(|top| top.country().is_empty()));
            let top: Vec<String> = (top.into_iter())
                .flat_map(|top| &top.track)
                .filter_map(|track| uri("track", track.gid()))
                .collect();
            // The first album of a group is its current release; the rest are older editions.
            let albums: Vec<String> = (raw.album_group.iter())
                .chain(&raw.single_group)
                .filter_map(|group| group.album.first())
                .filter_map(|album| uri("album", album.gid()))
                .collect();
            let albums = self
                .batch::<metadata::Album>(&albums, ExtensionKind::ALBUM_V4)
                .await?;
            Ok(Artist {
                name: raw.name().into(),
                art: art(&raw.portrait_group),
                top_tracks: self.tracks(&top).await?,
                albums: albums.iter().filter_map(album).collect(),
            })
        })
    }

    fn playlist(&self, id: SpotifyId) -> Fetch<'_, Vec<PlaylistItem>> {
        Box::pin(async move {
            let raw = self.session.spclient().get_playlist(&librespot(id)).await?;
            let raw = SelectedListContent::parse_from_bytes(&raw)?;
            let contents = raw.contents.get_or_default();
            if contents.truncated() {
                tracing::warn!(%id, "spclient sent part of a playlist");
            }
            let mut added: HashMap<SpotifyId, i64> = HashMap::new();
            let mut tracks = Vec::new();
            for item in &contents.items {
                let Some(track) = item.uri().strip_prefix("spotify:track:") else {
                    continue;
                };
                if let Ok(track) = track.parse() {
                    added.entry(track).or_insert(item.attributes.timestamp());
                }
                tracks.push(item.uri().to_owned());
            }
            let tracks = self.tracks(&tracks).await?;
            Ok((tracks.into_iter())
                .map(|track| {
                    let added_at = (added.get(&track.id))
                        .filter(|&&ms| ms > 0)
                        .map(|&ms| iso_utc(ms));
                    PlaylistItem { track, added_at }
                })
                .collect())
        })
    }
}

/// When each playlist in the user's library last changed, in Unix milliseconds, from the
/// playlist list spclient keeps; playlists it gives no time for are left out.
pub(crate) async fn modified(session: &Session) -> Result<Vec<(SpotifyId, i64)>, Error> {
    let mut modified = Vec::new();
    let mut from = 0;
    loop {
        let endpoint = format!(
            "/playlist/v2/user/{}/rootlist?decorate=timestamp&from={from}&length={ROOTLIST_PAGE}",
            session.username()
        );
        let raw = (session.spclient())
            .request(&Method::GET, &endpoint, None, None)
            .await?;
        let raw = SelectedListContent::parse_from_bytes(&raw)?;
        let contents = raw.contents.get_or_default();
        modified.extend(
            (contents.items.iter())
                .zip(&contents.meta_items)
                .filter_map(|(item, meta)| {
                    let id = item.uri().strip_prefix("spotify:playlist:")?.parse().ok()?;
                    (meta.timestamp() > 0).then_some((id, meta.timestamp()))
                }),
        );
        from += contents.items.len();
        if !contents.truncated() || contents.items.is_empty() {
            return Ok(modified);
        }
    }
}

/// Unix milliseconds as ISO 8601 UTC text to the second, the form the Web API gives
/// times in, so stored times compare as text.
fn iso_utc(ms: i64) -> Box<str> {
    let seconds = ms.div_euclid(1000);
    let (days, time) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days to a civil date, after Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
    .into()
}

fn librespot(id: SpotifyId) -> LibrespotId {
    LibrespotId { id: id.0 }
}

/// The ID in a raw 16-byte `gid`.
fn id(gid: &[u8]) -> Option<SpotifyId> {
    Some(SpotifyId(u128::from_be_bytes(gid.try_into().ok()?)))
}

fn uri(kind: &str, gid: &[u8]) -> Option<String> {
    Some(format!("spotify:{kind}:{}", id(gid)?))
}

fn track(raw: &metadata::Track) -> Option<Track> {
    Some(Track {
        id: id(raw.gid())?,
        name: raw.name().into(),
        artists: credits(&raw.artist),
        duration_ms: u32::try_from(raw.duration()).unwrap_or(0),
        art: art(&raw.album.cover_group),
    })
}

fn album(raw: &metadata::Album) -> Option<Album> {
    Some(Album {
        id: id(raw.gid())?,
        name: raw.name().into(),
        artists: credits(&raw.artist),
        year: (raw.date.as_ref())
            .filter(|date| date.has_year())
            .and_then(|date| u16::try_from(date.year()).ok()),
        kind: match raw.type_() {
            Type::SINGLE | Type::EP => AlbumKind::Single,
            Type::COMPILATION => AlbumKind::Compilation,
            Type::ALBUM | Type::AUDIOBOOK | Type::PODCAST => AlbumKind::Album,
        },
        art: art(&raw.cover_group),
        tracks: Vec::new(),
    })
}

fn credits(artists: &[metadata::Artist]) -> Vec<Credit> {
    (artists.iter())
        .filter_map(|artist| {
            Some(Credit {
                id: id(artist.gid())?,
                name: artist.name().into(),
                art: Art::default(),
            })
        })
        .collect()
}

/// The sizes of a picture as spclient lists them; a size without a width has the width
/// Spotify serves it at.
fn art(group: &metadata::ImageGroup) -> Art {
    let sizes: Vec<(u32, String)> = (group.image.iter())
        .map(|image| {
            let width = u32::try_from(image.width())
                .ok()
                .filter(|&width| width > 0)
                .unwrap_or(match image.size() {
                    Size::SMALL => 64,
                    Size::DEFAULT => 300,
                    Size::LARGE => 640,
                    Size::XLARGE => 1280,
                });
            let mut url = IMAGE_CDN.to_owned();
            for byte in image.file_id() {
                url.push(char::from(HEX[usize::from(byte >> 4)]));
                url.push(char::from(HEX[usize::from(byte & 0xf)]));
            }
            (width, url)
        })
        .collect();
    Art::new(
        sizes
            .iter()
            .map(|(width, url)| (Some(*width), url.as_str())),
    )
}
