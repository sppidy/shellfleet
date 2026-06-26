use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use shared::Message;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;

pub struct TerminalSession {
    pub tx_resize: std::sync::mpsc::Sender<(u16, u16)>,
    pub tx_input: std::sync::mpsc::Sender<Vec<u8>>,
}

/// Pick a usable interactive shell on Unix hosts. Honours the operator's
/// `$SHELL` when it points at a real executable, then falls back bash → sh
/// so the terminal still opens on minimal hosts (Alpine, busybox, slim
/// containers) that ship no bash. `bash`/`$SHELL` get `-l` (login profile);
/// plain `sh` gets `-i`, which every POSIX sh accepts (`-l` is not portable
/// across dash/ash).
#[cfg(not(target_os = "windows"))]
fn pick_login_shell() -> (String, Vec<&'static str>) {
    if let Ok(sh) = std::env::var("SHELL") {
        if !sh.is_empty() && std::path::Path::new(&sh).exists() {
            return (sh, vec!["-l"]);
        }
    }
    for cand in ["/bin/bash", "/usr/bin/bash"] {
        if std::path::Path::new(cand).exists() {
            return (cand.to_string(), vec!["-l"]);
        }
    }
    for cand in ["/bin/sh", "/usr/bin/sh"] {
        if std::path::Path::new(cand).exists() {
            return (cand.to_string(), vec!["-i"]);
        }
    }
    // Last resort: let PATH resolution try plain `bash`.
    ("bash".to_string(), vec!["-l"])
}

pub fn spawn_terminal(
    session_id: String,
    tx_msg: mpsc::UnboundedSender<Message>,
) -> Result<TerminalSession, String> {
    #[cfg(target_os = "windows")]
    let cmd = CommandBuilder::new("powershell.exe");
    #[cfg(not(target_os = "windows"))]
    let cmd = {
        let (shell, args) = pick_login_shell();
        let mut c = CommandBuilder::new(shell);
        c.args(args);
        c
    };
    spawn_pty(cmd, session_id, tx_msg)
}

/// Open a PTY for `docker exec -it <container_id> <shell>`. Reuses the same
/// stdin/stdout pump as the host terminal — the only difference is the
/// command being launched. The agent doesn't keep this around at idle:
/// the modal closes → `tx_input` is dropped → write thread exits → the
/// docker-exec child gets EOF on stdin and reaps.
pub fn spawn_docker_exec(
    container_id: &str,
    shell: &str,
    tx_msg: mpsc::UnboundedSender<Message>,
) -> Result<TerminalSession, String> {
    // Container-exec sessions tag emitted TerminalData with an empty
    // session_id; the dashboard's exec terminal listens for "".
    let session_id = String::new();
    // Validate so a `container_id` like "--privileged" can't slip in
    // as a docker flag. Docker container IDs are 12 or 64 hex chars in
    // their canonical form, but names allow `[a-zA-Z0-9][a-zA-Z0-9_.-]*`.
    if container_id.is_empty() || container_id.starts_with('-') || container_id.len() > 256 {
        return Err("invalid container id".to_string());
    }
    if !container_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err("container id has disallowed characters".to_string());
    }
    let shell = if shell.is_empty() { "sh" } else { shell };
    // Likewise restrict the shell argument — there's no use case for
    // anything outside a tiny set, and a value like `-c` followed by
    // arbitrary code would be hostile.
    if shell.starts_with('-')
        || !shell
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
    {
        return Err("invalid shell".to_string());
    }
    let mut cmd = CommandBuilder::new("docker");
    cmd.args(["exec", "-it", container_id, shell]);
    spawn_pty(cmd, session_id, tx_msg)
}

fn spawn_pty(
    cmd: CommandBuilder,
    session_id: String,
    tx_msg: mpsc::UnboundedSender<Message>,
) -> Result<TerminalSession, String> {
    let pty_system = NativePtySystem::default();

    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;

    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let (tx_resize, rx_resize) = std::sync::mpsc::channel::<(u16, u16)>();
    let (tx_input, rx_input) = std::sync::mpsc::channel::<Vec<u8>>();

    let master_ref = Arc::new(Mutex::new(pair.master));

    // Resize thread
    let master_resize = Arc::clone(&master_ref);
    thread::spawn(move || {
        while let Ok((cols, rows)) = rx_resize.recv() {
            if let Ok(m) = master_resize.lock() {
                let _ = m.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
    });

    // Write thread
    thread::spawn(move || {
        while let Ok(data) = rx_input.recv() {
            let _ = writer.write_all(&data);
        }
    });

    // Read thread
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let _ = tx_msg.send(Message::TerminalData {
                        session_id: session_id.clone(),
                        data: buf[..n].to_vec(),
                    });
                }
                _ => break, // EOF or Error
            }
        }
        let _ = child.wait();
    });

    Ok(TerminalSession {
        tx_resize,
        tx_input,
    })
}
