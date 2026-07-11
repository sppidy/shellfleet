pub mod privileged;

/// Bounded sender for messages headed to the control-plane WebSocket.
/// Producers use non-blocking `try_send`: when the connection cannot keep up,
/// a producer sees an error instead of growing an unbounded in-memory queue.
#[derive(Clone)]
pub struct Outgoing(tokio::sync::mpsc::Sender<shared::Message>);

/// Why a non-blocking control-plane send was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutgoingSendError {
    /// The bounded queue is full, so dropping this response is safer than
    /// allowing producer work to accumulate in memory.
    Full,
    /// The WebSocket writer has stopped and no longer accepts messages.
    Closed,
}

impl Outgoing {
    pub fn new(sender: tokio::sync::mpsc::Sender<shared::Message>) -> Self {
        Self(sender)
    }

    pub fn send(&self, message: shared::Message) -> Result<(), OutgoingSendError> {
        self.0.try_send(message).map_err(|error| match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => OutgoingSendError::Full,
            tokio::sync::mpsc::error::TrySendError::Closed(_) => OutgoingSendError::Closed,
        })
    }
}
