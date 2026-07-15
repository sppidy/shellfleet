use crate::AppState;
use shared::Message;
use std::{sync::Arc, time::Duration};

fn collection_requests(capabilities: &[String]) -> Vec<Message> {
    let legacy = capabilities.is_empty();
    let has = |value: &str| capabilities.iter().any(|item| item == value);
    let mut requests = vec![Message::SystemStatsRequest];
    if legacy || has("systemd") {
        requests.push(Message::ListServicesRequest);
    }
    if legacy || has("docker") {
        requests.push(Message::DockerListRequest);
    }
    if legacy || has("swarm") {
        requests.push(Message::SwarmListRequest);
    }
    requests
}

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let agents = state
                .agents
                .lock()
                .await
                .values()
                .map(|entry| (entry.tx.clone(), entry.capabilities.clone()))
                .collect::<Vec<_>>();
            for (tx, capabilities) in agents {
                for request in collection_requests(&capabilities) {
                    let _ = tx.send(request);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::Message;

    #[test]
    fn collector_requests_only_advertised_capabilities() {
        let requests = collection_requests(&["systemd".into()]);
        assert_eq!(requests.len(), 2);
        assert!(matches!(requests[0], Message::SystemStatsRequest));
        assert!(matches!(requests[1], Message::ListServicesRequest));
    }

    #[test]
    fn legacy_agents_receive_all_read_requests() {
        let requests = collection_requests(&[]);
        assert_eq!(requests.len(), 4);
        assert!(
            requests
                .iter()
                .any(|message| matches!(message, Message::DockerListRequest))
        );
        assert!(
            requests
                .iter()
                .any(|message| matches!(message, Message::SwarmListRequest))
        );
    }
}
