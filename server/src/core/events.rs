use super::model::{CoreEvent, CoreEventKind};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::broadcast;

struct Inner {
    tx: broadcast::Sender<CoreEvent>,
    next_id: AtomicU64,
}

#[derive(Clone)]
pub struct CoreEventBus {
    inner: Arc<Inner>,
}

impl CoreEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            inner: Arc::new(Inner {
                tx,
                next_id: AtomicU64::new(0),
            }),
        }
    }

    pub fn publish(&self, kind: CoreEventKind, agent_id: Option<&str>, observed_at: i64) {
        let event = CoreEvent {
            id: self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1,
            kind,
            agent_id: agent_id.map(str::to_owned),
            observed_at,
        };
        let _ = self.inner.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.inner.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn published_events_have_monotonic_ids() {
        let bus = CoreEventBus::new(8);
        let mut receiver = bus.subscribe();
        bus.publish(CoreEventKind::HostUpdated, Some("node-a-id"), 100);
        bus.publish(CoreEventKind::HostUpdated, Some("node-a-id"), 101);
        assert_eq!(receiver.recv().await.unwrap().id, 1);
        assert_eq!(receiver.recv().await.unwrap().id, 2);
    }
}
