use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Why a call to an actor got no reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,
    #[error("the service stopped")]
    Stopped,
}

/// The reply slot of a request to an actor, with the token its caller cancels it by.
#[derive(Debug)]
pub struct Req<T> {
    reply: oneshot::Sender<T>,
    cancel: CancellationToken,
}

impl<T> Req<T> {
    /// Whether the caller cancelled or stopped waiting, so the work can be skipped.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled() || self.reply.is_closed()
    }

    /// Answers the caller.
    pub fn reply(self, value: T) {
        // A caller that cancelled or stopped waiting no longer needs the answer.
        let _ = self.reply.send(value);
    }
}

/// Sends an actor the message `make` builds around a fresh `Req` and waits for the reply,
/// or until `cancel` fires.
pub async fn call<M, T, E: From<Error>>(
    tx: &mpsc::Sender<M>,
    cancel: CancellationToken,
    make: impl FnOnce(Req<Result<T, E>>) -> M,
) -> Result<T, E> {
    let (reply, rx) = oneshot::channel();
    let message = make(Req {
        reply,
        cancel: cancel.clone(),
    });
    let answer = async {
        if tx.send(message).await.is_err() {
            return Err(Error::Stopped.into());
        }
        rx.await.unwrap_or_else(|_| Err(Error::Stopped.into()))
    };
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(Error::Cancelled.into()),
        answer = answer => answer,
    }
}
