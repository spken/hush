use model::{Entity, Hits, web::Page};
use store::Store;
use tokio_util::sync::CancellationToken;
use webapi::ApiGateway;

use crate::{Error, sync::Save, web_api};

/// Searches Spotify's catalog for `query`: a page of each of `kinds` from `offset`. Stores
/// the entities it finds and returns their keys per kind.
pub async fn search(
    gateway: &ApiGateway,
    store: &Store,
    query: &str,
    kinds: &[Entity],
    offset: u32,
    cancel: CancellationToken,
) -> Result<Vec<(Entity, Hits)>, Error> {
    let found = gateway.search(query, kinds, offset, cancel).await?;
    let kinds = [
        (
            Entity::Track,
            hits(
                store,
                found.tracks,
                offset,
                web_api::track,
                &Store::save_tracks,
            )
            .await?,
        ),
        (
            Entity::Album,
            hits(
                store,
                found.albums,
                offset,
                web_api::album,
                &Store::save_albums,
            )
            .await?,
        ),
        (
            Entity::Artist,
            hits(
                store,
                found.artists,
                offset,
                web_api::credit,
                &Store::save_artists,
            )
            .await?,
        ),
        (
            Entity::Playlist,
            hits(
                store,
                found.playlists,
                offset,
                |playlist| playlist.id.is_some().then_some(playlist),
                &Store::save_playlists,
            )
            .await?,
        ),
    ];
    Ok((kinds.into_iter())
        .filter_map(|(entity, hits)| Some((entity, hits?)))
        .collect())
}

/// Stores the entities of a page of results that started at `offset`; `None` when Spotify
/// sent no page for the kind.
async fn hits<T, U>(
    store: &Store,
    page: Option<Page<Option<T>>>,
    offset: u32,
    item: impl FnMut(T) -> Option<U>,
    save: &impl Save<U>,
) -> Result<Option<Hits>, Error> {
    let Some(page) = page else {
        return Ok(None);
    };
    let sent = u32::try_from(page.items.len()).unwrap_or(u32::MAX);
    let items = page.items.into_iter().flatten().filter_map(item).collect();
    Ok(Some(Hits {
        keys: save(store, items).await?,
        next: offset.saturating_add(sent),
        total: page.total.unwrap_or(0),
    }))
}
