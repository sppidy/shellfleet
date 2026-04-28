//! `kubectl exec`-style PTY into a pod's container.
//!
//! Reuses the v14 TerminalData / TerminalResize / StopTerminalRequest
//! variants for the byte stream — the dashboard's existing xterm.js
//! plumbing was already keyed by `session_id`, so the same code path
//! that drives a host shell drives a kube exec without changes.
//!
//! Lifecycle:
//!   1. K8sExecRequest arrives → `spawn_exec` builds an
//!      `Api::exec(...)` AttachedProcess and three pump tasks.
//!   2. K8sExecResponse goes back synchronously with success/err.
//!   3. Stdout pump → TerminalData; client TerminalData → stdin;
//!      TerminalResize → AttachedProcess.terminal_size().
//!   4. StopTerminalRequest with the same session_id aborts the
//!      pumps; the agent map drop closes the senders, kube-rs reaps
//!      the underlying SPDY/WebSocket stream.

use futures_util::SinkExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{AttachParams, TerminalSize};
use kube::{Api, Client};
use shared::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub struct K8sExecSession {
    pub tx_input: mpsc::UnboundedSender<Vec<u8>>,
    pub tx_resize: mpsc::UnboundedSender<(u16, u16)>,
    pub supervisor: JoinHandle<()>,
}

impl K8sExecSession {
    pub fn abort(self) {
        self.supervisor.abort();
    }
}

pub struct ExecArgs {
    pub session_id: String,
    pub namespace: String,
    pub pod_name: String,
    pub container: Option<String>,
    pub command: Vec<String>,
}

pub async fn spawn_exec(
    args: ExecArgs,
    tx_msg: mpsc::UnboundedSender<Message>,
) -> Result<K8sExecSession, String> {
    let client = Client::try_default()
        .await
        .map_err(|e| format!("kube client: {e}"))?;
    let api: Api<Pod> = Api::namespaced(client, &args.namespace);

    let mut params = AttachParams::default()
        .stdin(true)
        .stdout(true)
        .stderr(false)
        .tty(true);
    if let Some(c) = args.container.as_deref() {
        params = params.container(c);
    }

    let cmd: Vec<String> = if args.command.is_empty() {
        vec!["/bin/sh".to_string()]
    } else {
        args.command.clone()
    };

    let mut attached = api
        .exec(&args.pod_name, &cmd, &params)
        .await
        .map_err(|e| format!("exec: {e}"))?;

    let mut stdout = attached
        .stdout()
        .ok_or_else(|| "AttachedProcess has no stdout".to_string())?;
    let mut stdin = attached
        .stdin()
        .ok_or_else(|| "AttachedProcess has no stdin".to_string())?;
    let mut size_sender = attached
        .terminal_size()
        .ok_or_else(|| "AttachedProcess has no terminal_size handle".to_string())?;

    let (tx_input, mut rx_input) = mpsc::unbounded_channel::<Vec<u8>>();
    let (tx_resize, mut rx_resize) = mpsc::unbounded_channel::<(u16, u16)>();

    let session_id = args.session_id.clone();

    // stdout → TerminalData
    let session_id_read = session_id.clone();
    let tx_msg_read = tx_msg.clone();
    let read_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tx_msg_read
                        .send(Message::TerminalData {
                            session_id: session_id_read.clone(),
                            data: buf[..n].to_vec(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // rx_input → stdin
    let write_handle = tokio::spawn(async move {
        while let Some(data) = rx_input.recv().await {
            if stdin.write_all(&data).await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
    });

    // rx_resize → AttachedProcess.terminal_size
    let resize_handle = tokio::spawn(async move {
        while let Some((cols, rows)) = rx_resize.recv().await {
            if size_sender
                .send(TerminalSize {
                    width: cols,
                    height: rows,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Supervisor: when read exits (container EOF or operator stop),
    // tear down the rest, then surface the apiserver-side status so
    // the dashboard isn't left staring at a blank pane when e.g. the
    // container is distroless and /bin/sh never started.
    let session_id_super = session_id.clone();
    let tx_msg_super = tx_msg.clone();
    let supervisor = tokio::spawn(async move {
        let _ = read_handle.await;
        write_handle.abort();
        resize_handle.abort();
        let join_result = attached.join().await;
        let banner = match join_result {
            Ok(()) => "\r\n\x1b[2m[ session ended ]\x1b[0m\r\n".to_string(),
            Err(e) => format!("\r\n\x1b[31m[ exec failed: {e} ]\x1b[0m\r\n"),
        };
        let _ = tx_msg_super.send(Message::TerminalData {
            session_id: session_id_super,
            data: banner.into_bytes(),
        });
    });

    Ok(K8sExecSession {
        tx_input,
        tx_resize,
        supervisor,
    })
}
