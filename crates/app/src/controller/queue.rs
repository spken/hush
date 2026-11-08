//! The Controller's queue side: adding tracks to the queue and, while the queue panel is
//! open, showing what plays next. Both go through the Web API, which passes additions on
//! to whichever device plays, Hush included.

use std::time::Duration;

use model::Entity;
use tokio::time;
use tokio_util::sync::CancellationToken;
use webapi::ApiGateway;

use super::Controller;
use crate::view::View;

/// Whether the queue panel is on show, so the queue is read as it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Panel {
    Closed,
    Open,
}

/// How long after the queue changed it is read again, so Spotify reports the change.
pub(super) const QUEUE_SETTLE: Duration = Duration::from_secs(1);

impl Controller {
    /// Opens the queue panel and shows the queue, or with `false` closes it.
    pub(super) fn show_queue(&mut self, open: bool) {
        self.queue = if open { Panel::Open } else { Panel::Closed };
        self.read_queue(Duration::ZERO);
    }

    /// Adds the track with key `key` to the end of the queue, then shows the queue again.
    pub(super) async fn add_to_queue(&self, key: u32) -> Result<(), store::Error> {
        let Some(library) = &self.library else {
            tracing::info!("the queue needs the Web API");
            return Ok(());
        };
        let Some(id) = self.store.id(Entity::Track, key).await? else {
            tracing::warn!(key, "no such track to queue");
            return Ok(());
        };
        let gateway = library.gateway.clone();
        let view = (self.queue == Panel::Open).then(|| self.view.clone());
        tokio::spawn(async move {
            if let Err(error) = gateway.add_to_queue(id, CancellationToken::new()).await {
                tracing::warn!(%error, "track not queued");
                return;
            }
            if let Some(view) = view {
                show(&gateway, &view, QUEUE_SETTLE).await;
            }
        });
        Ok(())
    }

    /// Shows the queue after `delay` while its panel is open.
    pub(super) fn read_queue(&self, delay: Duration) {
        if self.queue == Panel::Closed {
            return;
        }
        let Some(library) = &self.library else { return };
        let gateway = library.gateway.clone();
        let view = self.view.clone();
        tokio::spawn(async move { show(&gateway, &view, delay).await });
    }
}

/// Reads the queue after `delay` and shows it; nothing plays next when no device is active.
async fn show(gateway: &ApiGateway, view: &View, delay: Duration) {
    time::sleep(delay).await;
    match gateway.queue(CancellationToken::new()).await {
        Ok(queue) => view.queue(queue.map(|queue| queue.queue).unwrap_or_default()),
        Err(error) => tracing::warn!(%error, "queue not read"),
    }
}
