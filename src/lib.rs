pub mod privileged;

/// Bounded sender for messages headed to the control-plane WebSocket.
/// Producers use non-blocking `try_send`: when the connection cannot keep up,
/// a producer sees an error instead of growing an unbounded in-memory queue.
#[derive(Clone)]
pub struct Outgoing(tokio::sync::mpsc::Sender<shared::Message>);

impl Outgoing {
    pub fn new(sender: tokio::sync::mpsc::Sender<shared::Message>) -> Self {
        Self(sender)
    }

    pub fn send(
        &self,
        message: shared::Message,
    ) -> Result<(), tokio::sync::mpsc::error::TrySendError<shared::Message>> {
        self.0.try_send(message)
    }
}
