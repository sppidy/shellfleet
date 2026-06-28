//! Shared dispatch for RunCommandRequest, used by both the API v1 exec
//! handler and the EE internal `/internal/exec-command` handler. Both
//! get the same oneshot-based waiter with cancellation-safe cleanup.

use std::sync::Arc;
use tokio::sync::oneshot;

use crate::AppState;

pub enum DispatchError {
    AgentNotFound,
    AgentDisconnected,
}

pub struct PendingRunCommand {
    pub rx: oneshot::Receiver<shared::Message>,
    _guard: PendingExecGuard,
}

struct PendingExecGuard {
    state: Arc<AppState>,
    request_id: String,
}

impl Drop for PendingExecGuard {
    fn drop(&mut self) {
        let rid = std::mem::take(&mut self.request_id);
        if rid.is_empty() {
            return;
        }
        let state = self.state.clone();
        tokio::spawn(async move {
            state.pending_exec.lock().await.remove(&rid);
        });
    }
}

pub async fn dispatch_run_command(
    state: &Arc<AppState>,
    agent_id: &str,
    command: String,
    timeout_secs: u64,
) -> Result<PendingRunCommand, DispatchError> {
    let entry = {
        let agents = state.agents.lock().await;
        agents.get(agent_id).cloned()
    };
    let Some(entry) = entry else {
        return Err(DispatchError::AgentNotFound);
    };

    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx_os, rx_os) = oneshot::channel();
    state.pending_exec.lock().await.insert(request_id.clone(), (agent_id.to_string(), tx_os));

    let _guard = PendingExecGuard {
        state: state.clone(),
        request_id: request_id.clone(),
    };

    if entry
        .tx
        .send(shared::Message::RunCommandRequest {
            request_id: request_id.clone(),
            command,
            timeout_secs,
        })
        .is_err()
    {
        return Err(DispatchError::AgentDisconnected);
    }

    Ok(PendingRunCommand {
        rx: rx_os,
        _guard,
    })
}
