use librespot_core::SpotifyUri;
use librespot_metadata::audio::{AudioItem, UniqueFields};
use librespot_playback::player::PlayerEvent;
use model::{Event, SpotifyId, Track, TrackId};

/// Translates a librespot player event, or returns `None` for events Hush does not use.
pub(crate) fn map(event: PlayerEvent) -> Option<Event> {
    match event {
        PlayerEvent::TrackChanged { audio_item } => track(*audio_item).map(Event::TrackChanged),
        PlayerEvent::Playing { position_ms, .. } => Some(Event::Playing { position_ms }),
        PlayerEvent::Paused { position_ms, .. } => Some(Event::Paused { position_ms }),
        PlayerEvent::Seeked { position_ms, .. }
        | PlayerEvent::PositionCorrection { position_ms, .. } => {
            Some(Event::Seeked { position_ms })
        }
        PlayerEvent::Stopped { .. } => Some(Event::Stopped),
        PlayerEvent::VolumeChanged { volume } => Some(Event::Volume(volume)),
        PlayerEvent::SessionConnected { .. } => Some(Event::Active(true)),
        PlayerEvent::SessionDisconnected { .. } => Some(Event::Active(false)),
        _ => None,
    }
}

/// Converts a loaded item into a `Track`; episodes and local files are not tracks.
fn track(item: AudioItem) -> Option<Track> {
    let (SpotifyUri::Track { id }, UniqueFields::Track { artists, .. }) =
        (item.track_id, item.unique_fields)
    else {
        return None;
    };
    Some(Track {
        id: TrackId(SpotifyId(id.id)),
        name: item.name.into(),
        artists: artists
            .0
            .into_iter()
            .map(|artist| artist.name.into())
            .collect(),
        duration_ms: item.duration_ms,
    })
}
