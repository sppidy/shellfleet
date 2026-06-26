//! General-purpose journalctl streamer.
//!
//! Differs from `journal.rs` (single-unit `journalctl -fu <unit>`)
//! in three ways:
//!
//!   1. **No mandatory unit.** Caller can pass zero, one, or many
//!      `-u <unit>` filters, plus priority / identifier / grep / since.
//!   2. **Streams are keyed by `stream_id`** (minted by the dashboard)
//!      rather than by unit name, so multiple concurrent streams from
//!      the same operator don't collide on a single host.
//!   3. **Lines are batched** into chunks (~100 lines or 250 ms,
//!      whichever first) to keep WS framing cost bounded on busy
//!      hosts. The single-line-per-message shape in `journal.rs` is
//!      fine for one unit but melts a 10k lines/sec node.
//!
//! ## Argv hardening
//!
//! Every operator-supplied filter is validated against a tight
//! character allowlist before going into argv, and we never invoke
//! a shell. Without this, a unit name like `--user` or a since
//! string like `; rm -rf /` would either be re-read by `journalctl`
//! as a flag or, in shell-out land, executed.
//!
//! Validation rules:
//!
//!   * No argument may start with `-`.
//!   * units / identifier — `[A-Za-z0-9._@:-\\]+`, max 256 chars.
//!   * priority — exactly one of the eight RFC-5424 names.
//!   * since — `[A-Za-z0-9 :+,.\-]+`, max 64 chars.
//!   * grep — passed via `journalctl -g <regex>`. journalctl handles
//!     the regex itself; we cap length at 256 chars.

use shared::Message;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

const ALLOWED_PRIORITIES: &[&str] = &[
    "emerg", "alert", "crit", "err", "warning", "notice", "info", "debug",
];
const MAX_LINES_PER_CHUNK: usize = 100;
const FLUSH_INTERVAL_MS: u64 = 250;

#[derive(Default, Clone)]
pub struct JournalStreams {
    inner: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

#[allow(clippy::too_many_arguments)]
pub struct StreamArgs {
    pub stream_id: String,
    pub units: Vec<String>,
    pub priority: Option<String>,
    pub since: Option<String>,
    pub grep: Option<String>,
    pub identifier: Option<String>,
    pub lines: u32,
    pub follow: bool,
}

impl JournalStreams {
    pub async fn start(&self, args: StreamArgs, tx: mpsc::UnboundedSender<Message>) {
        let stream_id = args.stream_id.clone();
        self.stop(&stream_id).await;
        let handle = tokio::spawn(async move {
            run_stream(args, tx).await;
        });
        self.inner.lock().await.insert(stream_id, handle);
    }

    pub async fn stop(&self, stream_id: &str) {
        if let Some(h) = self.inner.lock().await.remove(stream_id) {
            h.abort();
        }
    }
}

fn is_safe_token(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') || s.len() > 256 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '\\'))
}

fn is_safe_since(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') || s.len() > 64 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | ':' | '+' | ',' | '.' | '-'))
}

fn build_argv(args: &StreamArgs) -> Result<Vec<String>, String> {
    let mut argv = vec!["--no-pager".to_string(), "--output=short-iso".to_string()];
    if args.follow {
        argv.push("--follow".to_string());
    }
    let lines = args.lines.min(10_000); // hard cap on backlog
    argv.push(format!("--lines={lines}"));

    for unit in &args.units {
        if !is_safe_token(unit) {
            return Err(format!("invalid unit name: {unit}"));
        }
        argv.push("-u".to_string());
        argv.push(unit.clone());
    }
    if let Some(p) = args.priority.as_deref() {
        if !ALLOWED_PRIORITIES.contains(&p) {
            return Err(format!("invalid priority: {p}"));
        }
        argv.push("-p".to_string());
        argv.push(p.to_string());
    }
    if let Some(id) = args.identifier.as_deref() {
        if !is_safe_token(id) {
            return Err(format!("invalid identifier: {id}"));
        }
        argv.push("-t".to_string());
        argv.push(id.to_string());
    }
    if let Some(s) = args.since.as_deref() {
        if !is_safe_since(s) {
            return Err(format!("invalid since value: {s}"));
        }
        argv.push("--since".to_string());
        argv.push(s.to_string());
    }
    if let Some(g) = args.grep.as_deref() {
        if g.is_empty() || g.len() > 256 || g.starts_with('-') {
            return Err("invalid grep pattern".to_string());
        }
        argv.push("-g".to_string());
        argv.push(g.to_string());
    }
    Ok(argv)
}

async fn run_stream(args: StreamArgs, tx: mpsc::UnboundedSender<Message>) {
    let argv = match build_argv(&args) {
        Ok(a) => a,
        Err(e) => {
            let _ = tx.send(Message::JournalStreamEnd {
                stream_id: args.stream_id,
                error: Some(e),
            });
            return;
        }
    };

    let mut cmd = Command::new("journalctl");
    cmd.args(&argv);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Message::JournalStreamEnd {
                stream_id: args.stream_id,
                error: Some(format!("journalctl spawn: {e}")),
            });
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stream_id = args.stream_id.clone();

    // Channel for ALL output (stdout merged with stderr — operators
    // generally want them interleaved when scanning logs). The
    // batcher coalesces lines into JournalStreamChunk messages.
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();

    let stdout_task = stdout.map(|s| {
        let line_tx = line_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(s).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        })
    });
    let stderr_task = stderr.map(|s| {
        let line_tx = line_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(s).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        })
    });
    drop(line_tx); // close the original sender so the batcher exits with the children

    // Batcher: gathers up to MAX_LINES_PER_CHUNK or flushes every
    // FLUSH_INTERVAL_MS, whichever first.
    let tx_batch = tx.clone();
    let stream_id_batch = stream_id.clone();
    let batch_task = tokio::spawn(async move {
        let mut buf: Vec<String> = Vec::with_capacity(MAX_LINES_PER_CHUNK);
        let mut tick = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await; // drop the immediate first tick

        loop {
            tokio::select! {
                msg = line_rx.recv() => {
                    match msg {
                        Some(line) => {
                            buf.push(line);
                            if buf.len() >= MAX_LINES_PER_CHUNK {
                                let drained = std::mem::take(&mut buf);
                                if tx_batch
                                    .send(Message::JournalStreamChunk {
                                        stream_id: stream_id_batch.clone(),
                                        lines: drained,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        None => {
                            // All sources closed. Final flush.
                            if !buf.is_empty() {
                                let _ = tx_batch.send(Message::JournalStreamChunk {
                                    stream_id: stream_id_batch.clone(),
                                    lines: std::mem::take(&mut buf),
                                });
                            }
                            return;
                        }
                    }
                }
                _ = tick.tick() => {
                    if !buf.is_empty() {
                        let drained = std::mem::take(&mut buf);
                        if tx_batch
                            .send(Message::JournalStreamChunk {
                                stream_id: stream_id_batch.clone(),
                                lines: drained,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        }
    });

    let status = child.wait().await;
    if let Some(t) = stdout_task {
        let _ = t.await;
    }
    if let Some(t) = stderr_task {
        let _ = t.await;
    }
    let _ = batch_task.await;

    let error = match status {
        Ok(s) if s.success() => None,
        Ok(s) => Some(format!("journalctl exited with {:?}", s.code())),
        Err(e) => Some(format!("journalctl wait failed: {e}")),
    };
    let _ = tx.send(Message::JournalStreamEnd { stream_id, error });
}
