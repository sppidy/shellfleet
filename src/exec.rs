//! One-shot command execution for EE runbooks. Runs a command under `sh -c`
//! with a timeout, captures stdout/stderr + the exit code. Which commands may
//! run is gated upstream (EE's runbook allow-list + the CE ACL) — this just
//! executes what it's handed and reports the result.

use std::process::Stdio;
use tokio::process::Command;

pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

const MAX_OUTPUT: usize = 64 * 1024;

/// Truncate to at most `MAX_OUTPUT` bytes on a char boundary so a runaway
/// command can't blow the WS frame / server memory.
fn cap(bytes: Vec<u8>) -> String {
    let mut s = String::from_utf8_lossy(&bytes).into_owned();
    if s.len() > MAX_OUTPUT {
        let mut end = MAX_OUTPUT;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n…(output truncated)…");
    }
    s
}

pub async fn run(command: &str, timeout_secs: u64) -> ExecResult {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let dur = std::time::Duration::from_secs(timeout_secs.clamp(1, 3600));
    match tokio::time::timeout(dur, cmd.output()).await {
        Ok(Ok(out)) => ExecResult {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: cap(out.stdout),
            stderr: cap(out.stderr),
            error: None,
        },
        Ok(Err(e)) => ExecResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("spawn failed: {e}")),
        },
        Err(_) => ExecResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("command timed out after {timeout_secs}s")),
        },
    }
}
