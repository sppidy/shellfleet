use shared::Message;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

/// Hard cap on `journalctl --lines`. A viewer-allowed value of
/// `lines=u32::MAX` would force journald to emit the entire ring,
/// which on a busy host is hundreds of MB streamed over the WS.
/// 10_000 mirrors the `journal_stream` backlog cap.
const MAX_JOURNAL_LINES: u32 = 10_000;

/// Mirrors `journal_stream::is_safe_token`. systemd unit names use
/// `[A-Za-z0-9._@:\-\\]+`; rejects empty / leading-`-` / >256-char
/// values that could turn into journalctl flags or path-traversal
/// attempts when they hit argv.
fn valid_unit_name(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') || s.len() > 256 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '\\'))
}

/// One active `journalctl -fu <unit>` stream per unit. Restarting a
/// stream for the same unit cancels the previous one.
#[derive(Default, Clone)]
pub struct JournalStreams {
    inner: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl JournalStreams {
    pub async fn start(
        &self,
        unit: String,
        lines: u32,
        follow: bool,
        tx: mpsc::UnboundedSender<Message>,
    ) {
        // Reject hostile / malformed unit names before they land
        // in argv; cap the backlog before journald starts spooling.
        if !valid_unit_name(&unit) {
            let _ = tx.send(Message::JournalLogsEnd {
                unit,
                error: Some("invalid unit name".into()),
            });
            return;
        }
        let lines = lines.min(MAX_JOURNAL_LINES);

        self.stop(&unit).await;
        let unit_for_task = unit.clone();
        let tx_task = tx.clone();
        let handle = tokio::spawn(async move {
            run_stream(unit_for_task, lines, follow, tx_task).await;
        });
        self.inner.lock().await.insert(unit, handle);
    }

    pub async fn stop(&self, unit: &str) {
        if let Some(h) = self.inner.lock().await.remove(unit) {
            h.abort();
        }
    }
}

async fn run_stream(unit: String, lines: u32, follow: bool, tx: mpsc::UnboundedSender<Message>) {
    let mut cmd = Command::new("journalctl");
    cmd.arg("--no-pager");
    cmd.arg("--output=short-iso");
    if follow {
        cmd.arg("--follow");
    }
    cmd.arg(format!("--lines={lines}"));
    cmd.arg("-u").arg(&unit);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Message::JournalLogsEnd {
                unit,
                error: Some(format!("journalctl spawn: {e}")),
            });
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let unit_out = unit.clone();
    let unit_err = unit.clone();
    let tx_out = tx.clone();
    let tx_err = tx.clone();

    let stdout_task = stdout.map(|s| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(s).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_out
                    .send(Message::JournalLogsChunk {
                        unit: unit_out.clone(),
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
                    .send(Message::JournalLogsChunk {
                        unit: unit_err.clone(),
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
        Ok(s) => Some(format!("journalctl exited with {:?}", s.code())),
        Err(e) => Some(format!("journalctl wait failed: {e}")),
    };

    let _ = tx.send(Message::JournalLogsEnd { unit, error });
}
