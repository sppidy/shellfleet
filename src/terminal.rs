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

pub fn spawn_terminal(tx_msg: mpsc::UnboundedSender<Message>) -> Result<TerminalSession, String> {
    #[cfg(target_os = "windows")]
    let cmd = CommandBuilder::new("powershell.exe");
    #[cfg(not(target_os = "windows"))]
    let mut cmd = CommandBuilder::new("bash");
    #[cfg(not(target_os = "windows"))]
    cmd.args(["-l"]);
    spawn_pty(cmd, tx_msg)
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
    let mut cmd = CommandBuilder::new("docker");
    let shell = if shell.is_empty() { "sh" } else { shell };
    cmd.args(["exec", "-it", container_id, shell]);
    spawn_pty(cmd, tx_msg)
}

fn spawn_pty(
    cmd: CommandBuilder,
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
                        data: buf[..n].to_vec(),
                    });
                }
                _ => break, // EOF or Error
            }
        }
        let _ = child.wait();
    });

    Ok(TerminalSession { tx_resize, tx_input })
}
