//! One-shot command execution for EE runbooks and API exec. Runs a command
//! under `sh -c` with a timeout, captures stdout/stderr + exit code with a
//! bounded byte budget. Which commands may run is gated upstream — this just
//! executes what it's handed and reports the result.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use shared::MAX_OUTPUT_BYTES;

pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub truncated: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
}

const READER_GRACE_SECS: u64 = 5;
const EXEC_ALLOWLIST_ENV: &str = "SHELLFLEET_EXEC_ALLOW_JSON";

fn parse_command_allowlist(raw: Option<&str>) -> Result<Vec<String>, String> {
    let raw = raw
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{EXEC_ALLOWLIST_ENV} is not configured"))?;
    let entries: Vec<String> = serde_json::from_str(raw)
        .map_err(|e| format!("{EXEC_ALLOWLIST_ENV} must be a JSON array of strings: {e}"))?;
    let entries: Vec<String> = entries
        .into_iter()
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect();
    if entries.is_empty() {
        return Err(format!("{EXEC_ALLOWLIST_ENV} must contain at least one command"));
    }
    Ok(entries)
}

fn configured_command_allowlist() -> Result<Vec<String>, String> {
    let raw = std::env::var(EXEC_ALLOWLIST_ENV).ok();
    parse_command_allowlist(raw.as_deref())
}

fn command_allowed(command: &str, allowlist: &[String]) -> bool {
    let command = command.trim();
    !command.is_empty() && allowlist.iter().any(|allowed| command == allowed)
}

pub fn enabled() -> bool {
    configured_command_allowlist().is_ok()
}

fn blocked_result(error: String) -> ExecResult {
    ExecResult {
        exit_code: -1,
        stdout: String::new(),
        stderr: String::new(),
        error: Some(error),
        truncated: false,
        timed_out: false,
        duration_ms: 0,
    }
}

// ── read_to_limit ────────────────────────────────────────────────────

async fn read_to_limit<R: AsyncRead + Unpin>(
    pipe: R,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    budget: Arc<AtomicUsize>,
    limited: Arc<AtomicBool>,
    limit_tx: mpsc::Sender<()>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(pipe);
    loop {
        let mut buf = vec![0u8; 8192];
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        let reserved = loop {
            let cur = budget.load(Ordering::Relaxed);
            let take = n.min(cur);
            if take == 0 {
                break 0;
            }
            match budget.compare_exchange_weak(cur, cur - take, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break take,
                Err(_) => continue,
            }
        };
        if reserved < n {
            let _ = tx.send(buf[..reserved].to_vec());
            limited.store(true, Ordering::Relaxed);
            let _ = limit_tx.try_send(());
            break;
        }
        let _ = tx.send(buf[..n].to_vec());
    }
    Ok(())
}

// ── kill_process ──────────────────────────────────────────────────────

fn kill_process(child: &mut tokio::process::Child, pgid: Option<i32>) {
    #[cfg(unix)]
    if let Some(gid) = pgid {
        unsafe { libc::kill(-gid, libc::SIGKILL); }
    }
    // Always call start_kill: catches non-Unix and Unix group-kill failure.
    let _ = child.start_kill();
}

// ── concatenate ───────────────────────────────────────────────────────

fn concatenate(rx: &mut mpsc::UnboundedReceiver<Vec<u8>>) -> String {
    let mut buf = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        if buf.len() + chunk.len() > MAX_OUTPUT_BYTES {
            buf.extend_from_slice(&chunk[..MAX_OUTPUT_BYTES - buf.len()]);
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

// ── run ───────────────────────────────────────────────────────────────

pub async fn run(command: &str, timeout_secs: u64) -> ExecResult {
    let allowlist = match configured_command_allowlist() {
        Ok(allowlist) => allowlist,
        Err(error) => return blocked_result(error),
    };
    if !command_allowed(command, &allowlist) {
        return blocked_result("command blocked by agent execution allow-list".to_string());
    }

    let dur = Duration::from_secs(timeout_secs.clamp(1, 3600));

    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    let timer = Instant::now();

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ExecResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(format!("spawn failed: {e}")),
                truncated: false,
                timed_out: false,
                duration_ms: timer.elapsed().as_millis() as u64,
            }
        }
    };

    #[cfg(unix)]
    let pgid = child.id().map(|p| p as i32);
    #[cfg(not(unix))]
    let pgid: Option<i32> = None;

    let budget = Arc::new(AtomicUsize::new(MAX_OUTPUT_BYTES));
    let limited = Arc::new(AtomicBool::new(false));
    let (limit_tx, mut limit_rx) = mpsc::channel::<()>(1);
    let (stdout_tx, mut stdout_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (stderr_tx, mut stderr_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let mut stdout_h = tokio::spawn(read_to_limit(
        child.stdout.take().unwrap(),
        stdout_tx.clone(),
        budget.clone(),
        limited.clone(),
        limit_tx.clone(),
    ));
    let mut stderr_h = tokio::spawn(read_to_limit(
        child.stderr.take().unwrap(),
        stderr_tx.clone(),
        budget.clone(),
        limited.clone(),
        limit_tx.clone(),
    ));

    drop(limit_tx);

    let deadline = tokio::time::sleep(dur);
    tokio::pin!(deadline);
    let mut exit_code: i32 = -1;
    let mut timed_out = false;
    let mut killed_for_limit = false;

    tokio::select! {
        status = child.wait() => {
            exit_code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
        }
        _ = &mut deadline => {
            timed_out = true;
            killed_for_limit = false;
            kill_process(&mut child, pgid);
        }
        Some(()) = limit_rx.recv() => {
            killed_for_limit = true;
            kill_process(&mut child, pgid);
        }
    }

    if timed_out || killed_for_limit {
        let _ = child.wait().await;
    }

    let grace = tokio::time::sleep(Duration::from_secs(READER_GRACE_SECS));
    tokio::pin!(grace);
    let mut stdout_joined = false;
    let mut stderr_joined = false;
    let mut stdout_err: Option<String> = None;
    let mut stderr_err: Option<String> = None;
    loop {
        tokio::select! {
            r = &mut stdout_h, if !stdout_joined => {
                stdout_joined = true;
                match r {
                    Ok(inner) => {
                        if let Err(e) = inner {
                            stdout_err = Some(format!("reader: {e}"));
                        }
                    }
                    Err(join) => stdout_err = Some(format!("join: {join}")),
                }
            }
            r = &mut stderr_h, if !stderr_joined => {
                stderr_joined = true;
                match r {
                    Ok(inner) => {
                        if let Err(e) = inner {
                            stderr_err = Some(format!("reader: {e}"));
                        }
                    }
                    Err(join) => stderr_err = Some(format!("join: {join}")),
                }
            }
            _ = &mut grace => { break; }
        }
        if stdout_joined && stderr_joined {
            break;
        }
    }
    let readers_aborted = !stdout_joined || !stderr_joined;
    let reader_failed = stdout_err.is_some() || stderr_err.is_some();
    if !stdout_joined {
        stdout_h.abort();
        let _ = stdout_h.await;
    }
    if !stderr_joined {
        stderr_h.abort();
        let _ = stderr_h.await;
    }

    if readers_aborted {
        #[cfg(unix)]
        if let Some(gid) = pgid {
            unsafe { libc::kill(-gid, libc::SIGKILL); }
        }
    }

    drop(stdout_tx);
    drop(stderr_tx);
    let stdout = concatenate(&mut stdout_rx);
    let stderr = concatenate(&mut stderr_rx);

    let truncated = limited.load(Ordering::Relaxed) || readers_aborted || reader_failed;

    let reader_error = match (stdout_err, stderr_err) {
        (Some(e1), Some(e2)) => Some(format!("stdout: {e1}, stderr: {e2}")),
        (Some(e), _) | (_, Some(e)) => Some(e),
        (None, None) => None,
    };

    ExecResult {
        exit_code,
        stdout,
        stderr,
        error: reader_error,
        truncated,
        timed_out,
        duration_ms: timer.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_allowlist_is_required_and_must_be_valid_json() {
        assert!(parse_command_allowlist(None).is_err());
        assert!(parse_command_allowlist(Some("")).is_err());
        assert!(parse_command_allowlist(Some("not-json")).is_err());
        assert!(parse_command_allowlist(Some("[]")).is_err());
    }

    #[test]
    fn execution_allowlist_matches_only_complete_commands() {
        let allow = parse_command_allowlist(Some(
            r#"["uptime","systemctl restart nginx"]"#,
        ))
        .unwrap();

        assert!(command_allowed("uptime", &allow));
        assert!(command_allowed("  systemctl restart nginx  ", &allow));
        assert!(!command_allowed("uptime; id", &allow));
        assert!(!command_allowed("systemctl restart nginx; id", &allow));
        assert!(!command_allowed("systemctl restart sshd", &allow));
    }
}
