//! Pod log streaming — mirrors `journal_stream.rs` for k8s.
//!
//! The dashboard mints a `stream_id` per Logs modal, the agent maps
//! it to a tokio task that pumps `kube::Api<Pod>::log_stream(...)`
//! into batched `K8sLogsChunk` messages, and a matching
//! `K8sLogsStop` aborts the task. Backpressure is implicit — the
//! sender is unbounded but the task drops if the dashboard goes
//! away (server detects the WS close, tx is dropped, send returns
//! Err and the loop exits).

use futures_util::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client, api::LogParams};
use shared::Message;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

const MAX_LINES_PER_CHUNK: usize = 100;
const FLUSH_INTERVAL_MS: u64 = 250;

#[derive(Default, Clone)]
pub struct K8sLogStreams {
    inner: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

pub struct LogArgs {
    pub stream_id: String,
    pub namespace: String,
    pub pod_name: String,
    pub container: Option<String>,
    pub tail_lines: i64,
    pub follow: bool,
}

impl K8sLogStreams {
    pub async fn start(&self, args: LogArgs, tx: mpsc::UnboundedSender<Message>) {
        let stream_id = args.stream_id.clone();
        // Re-issuing the same stream_id supersedes the prior task —
        // matches journal_stream behavior.
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

async fn run_stream(args: LogArgs, tx: mpsc::UnboundedSender<Message>) {
    let client = match Client::try_default().await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Message::K8sLogsEnd {
                stream_id: args.stream_id,
                error: Some(format!("kube client: {e}")),
            });
            return;
        }
    };
    let api: Api<Pod> = Api::namespaced(client, &args.namespace);

    let params = LogParams {
        container: args.container.clone(),
        follow: args.follow,
        tail_lines: if args.tail_lines > 0 {
            Some(args.tail_lines)
        } else {
            None
        },
        timestamps: true,
        ..Default::default()
    };

    let mut stream = match api.log_stream(&args.pod_name, &params).await {
        Ok(s) => s.lines(),
        Err(e) => {
            let _ = tx.send(Message::K8sLogsEnd {
                stream_id: args.stream_id,
                error: Some(format!("log_stream: {e}")),
            });
            return;
        }
    };

    let stream_id = args.stream_id.clone();
    let mut buf: Vec<String> = Vec::with_capacity(MAX_LINES_PER_CHUNK);
    let mut tick = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await; // drop the immediate first tick

    loop {
        tokio::select! {
            line = stream.next() => {
                match line {
                    Some(Ok(line)) => {
                        buf.push(line);
                        if buf.len() >= MAX_LINES_PER_CHUNK {
                            let drained = std::mem::take(&mut buf);
                            if tx
                                .send(Message::K8sLogsChunk {
                                    stream_id: stream_id.clone(),
                                    lines: drained,
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        if !buf.is_empty() {
                            let _ = tx.send(Message::K8sLogsChunk {
                                stream_id: stream_id.clone(),
                                lines: std::mem::take(&mut buf),
                            });
                        }
                        let _ = tx.send(Message::K8sLogsEnd {
                            stream_id,
                            error: Some(format!("stream: {e}")),
                        });
                        return;
                    }
                    None => {
                        if !buf.is_empty() {
                            let _ = tx.send(Message::K8sLogsChunk {
                                stream_id: stream_id.clone(),
                                lines: std::mem::take(&mut buf),
                            });
                        }
                        let _ = tx.send(Message::K8sLogsEnd {
                            stream_id,
                            error: None,
                        });
                        return;
                    }
                }
            }
            _ = tick.tick() => {
                if !buf.is_empty() {
                    let drained = std::mem::take(&mut buf);
                    if tx
                        .send(Message::K8sLogsChunk {
                            stream_id: stream_id.clone(),
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
}
