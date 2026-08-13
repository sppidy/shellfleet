use agent::Outgoing;
use shared::Message;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Hard cap on the docker `--tail` argument. A viewer-allowed
/// request with `tail=2_000_000_000` would force docker to read the
/// entire log file into RAM and stream it over the WS, blowing
/// agent memory and operator browser memory long before the
/// operator could cancel. 10_000 lines mirrors the
/// `journal_stream` backlog cap and is more than enough for the
/// "what just happened" use case the UI panel exists for.
const MAX_DOCKER_TAIL: u32 = 10_000;

/// Mirrors the docker container-id / name validation in
/// `terminal::spawn_docker_exec`. Names are
/// `[a-zA-Z0-9][a-zA-Z0-9_.-]*`, IDs are 12 or 64 hex chars; both
/// fit in this same allow-list. Without this check a leading `-`
/// could turn a request value into a flag (`--privileged` etc.)
/// when it lands in argv.
fn valid_container_id(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.len() <= 256
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// One active `docker logs` stream per container_id. New requests for the
/// same container cancel the previous stream so the operator never has to
/// stop one log before starting another.
#[derive(Default, Clone)]
pub struct LogStreams {
    inner: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl LogStreams {
    pub async fn start(&self, container_id: String, tail: u32, follow: bool, tx: Outgoing) {
        // Reject hostile / malformed container_ids before they
        // land in argv. The send goes to the same channel the
        // dashboard's logs panel listens on, so the operator
        // sees an error instead of a hang.
        if !valid_container_id(&container_id) {
            let _ = tx.send(Message::DockerLogsEnd {
                container_id,
                error: Some("invalid container id".into()),
            });
            return;
        }
        let tail = tail.min(MAX_DOCKER_TAIL);

        // Cancel any previous stream for the same container.
        self.stop(&container_id).await;

        let cid_for_task = container_id.clone();
        let tx_task = tx.clone();
        let handle = tokio::spawn(async move {
            run_stream(cid_for_task, tail, follow, tx_task).await;
        });
        self.inner.lock().await.insert(container_id, handle);
    }

    pub async fn stop(&self, container_id: &str) {
        if let Some(handle) = self.inner.lock().await.remove(container_id) {
            handle.abort();
        }
    }

    #[allow(dead_code)] // retained API for orderly teardown; not currently called
    pub async fn shutdown(&self) {
        let mut guard = self.inner.lock().await;
        for (_, h) in guard.drain() {
            h.abort();
        }
    }
}

async fn run_stream(container_id: String, tail: u32, follow: bool, tx: Outgoing) {
    let mut cmd = Command::new("docker");
    cmd.arg("logs");
    if follow {
        cmd.arg("--follow");
    }
    // `--timestamps` is a deliberate no-op — many users prefer raw output;
    // toggle later if a flag is added.
    cmd.arg(format!("--tail={tail}"));
    cmd.arg(&container_id);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Message::DockerLogsEnd {
                container_id,
                error: Some(format!("docker spawn: {e}")),
            });
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let cid_out = container_id.clone();
    let cid_err = container_id.clone();
    let tx_out = tx.clone();
    let tx_err = tx.clone();

    let stdout_task = stdout.map(|s| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(s).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_out
                    .send(Message::DockerLogsChunk {
                        container_id: cid_out.clone(),
                        data: line,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    });

    let stderr_task = stderr.map(|s| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(s).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_err
                    .send(Message::DockerLogsChunk {
                        container_id: cid_err.clone(),
                        data: line,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    });

    let status = child.wait().await;
    if let Some(t) = stdout_task {
        let _ = t.await;
    }
    if let Some(t) = stderr_task {
        let _ = t.await;
    }

    let error = match status {
        Ok(s) if s.success() => None,
        Ok(s) => Some(format!("docker logs exited with {:?}", s.code())),
        Err(e) => Some(format!("docker logs wait failed: {e}")),
    };

    let _ = tx.send(Message::DockerLogsEnd {
        container_id,
        error,
    });
}
