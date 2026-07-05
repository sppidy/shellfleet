use agent::privileged::{approval::GateState, crypto::Transport, framing, peer};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use shared::trusted::{TrustedClientFrame, TrustedHostFrame, TrustedOperation, TrustedPlaintext};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::net::{UnixListener, UnixStream};

const MAX_COMMAND_OUTPUT: usize = 8 * 1024 * 1024;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[tokio::main]
async fn main() {
    let state_dir = PathBuf::from(
        std::env::var("SHELLFLEET_GATE_STATE_DIR")
            .unwrap_or_else(|_| "/var/lib/shellfleet-gate".into()),
    );
    let approvers = PathBuf::from(
        std::env::var("SHELLFLEET_APPROVERS_PATH")
            .unwrap_or_else(|_| "/etc/shellfleet/approvers.json".into()),
    );
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--enroll-approver") {
        let id = args.get(2).unwrap_or_else(|| panic!("missing approver id"));
        let public_key = args
            .get(3)
            .unwrap_or_else(|| panic!("missing base64 approver public key"));
        enroll_approver(&approvers, id, public_key)
            .unwrap_or_else(|error| panic!("approver enrollment failed: {error}"));
        println!("enrolled approver {id}");
        return;
    }
    let socket = PathBuf::from(
        std::env::var("SHELLFLEET_GATE_SOCKET")
            .unwrap_or_else(|_| "/run/shellfleet/approval-gate.sock".into()),
    );
    let state = Arc::new(
        GateState::load_or_create(&state_dir, &approvers)
            .unwrap_or_else(|error| panic!("approval gate initialization failed: {error}")),
    );
    if args.get(1).map(String::as_str) == Some("--print-host-fingerprint") {
        use base64::Engine;
        use sha2::Digest;
        let digest = sha2::Sha256::digest(state.host_signing.verifying_key().to_bytes());
        println!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
        );
        return;
    }
    if socket.exists() {
        std::fs::remove_file(&socket).expect("remove stale approval-gate socket");
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).expect("create approval-gate socket directory");
    }
    let listener = UnixListener::bind(&socket).expect("bind approval-gate socket");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660))
            .expect("set approval-gate socket mode");
    }
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept approval-gate client");
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle(stream, state).await {
                eprintln!("approval gate request rejected: {error}");
            }
        });
    }
}

fn enroll_approver(path: &std::path::Path, id: &str, encoded: &str) -> Result<(), String> {
    use base64::Engine;
    #[cfg(unix)]
    if unsafe { libc::geteuid() } != 0 {
        return Err("approver enrollment must run as local root".into());
    }
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err("invalid approver id".into());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "invalid approver public-key encoding")?;
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| "approver public key must be exactly 32 bytes")?;
    ed25519_dalek::VerifyingKey::from_bytes(&key)
        .map_err(|_| "approver public key is not valid Ed25519")?;
    let mut values: std::collections::BTreeMap<String, String> = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    values.insert(id.to_owned(), encoded.to_owned());
    let temporary = path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&values).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    std::fs::rename(temporary, path).map_err(|error| error.to_string())
}

async fn handle(stream: UnixStream, state: Arc<GateState>) -> Result<(), String> {
    let uid = peer::peer_uid(&stream)?;
    let expected = peer::configured_agent_uid()?;
    if uid != expected {
        return Err(format!(
            "approval-gate peer uid {uid} is not agent uid {expected}"
        ));
    }
    let (mut reader, mut writer) = stream.into_split();
    let first = framing::read_frame(&mut reader).await?;
    let start = shared::trusted::decode_client(&first)?;
    let challenge = state.challenge(&start, now_unix())?;
    send_host(
        &mut writer,
        &TrustedHostFrame::Challenge(Box::new(challenge.clone())),
    )
    .await?;

    let approval = framing::read_frame(&mut reader).await?;
    let approval = shared::trusted::decode_client(&approval)?;
    let TrustedClientFrame::Approve {
        signed_manifest,
        approver_public,
        approver_signature,
    } = approval
    else {
        send_error(&mut writer, "expected signed approval").await?;
        return Err("expected signed approval".into());
    };
    if *signed_manifest != challenge {
        send_error(
            &mut writer,
            "approved manifest differs from broker challenge",
        )
        .await?;
        return Err("approved manifest differs from broker challenge".into());
    }
    let manifest = state.approve(
        &signed_manifest,
        approver_public,
        &approver_signature,
        now_unix(),
    )?;
    let canonical = manifest
        .canonical_bytes()
        .map_err(|error| error.to_string())?;
    let mut transport = Transport::broker(
        &state.transport_secret,
        manifest.client_ephemeral_public,
        &canonical,
        &manifest.request_id,
    )?;

    match manifest.operation {
        TrustedOperation::RootCommand {
            program,
            args,
            timeout_secs,
        } => {
            run_command(&mut writer, &mut transport, &program, &args, timeout_secs).await?;
        }
        TrustedOperation::RootPty {
            shell,
            ttl_secs,
            cols,
            rows,
        } => {
            run_pty(
                &mut reader,
                &mut writer,
                &mut transport,
                &shell,
                ttl_secs,
                cols,
                rows,
            )
            .await?;
        }
    }
    send_host(&mut writer, &TrustedHostFrame::Closed).await
}

async fn send_host<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &TrustedHostFrame,
) -> Result<(), String> {
    framing::write_frame(writer, &shared::trusted::encode_host(frame)?).await
}

async fn send_error<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    message: &str,
) -> Result<(), String> {
    send_host(
        writer,
        &TrustedHostFrame::Error {
            message: message.to_owned(),
        },
    )
    .await
}

async fn send_encrypted<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    transport: &mut Transport,
    plaintext: TrustedPlaintext,
) -> Result<(), String> {
    let encoded = serde_json::to_vec(&plaintext).map_err(|error| error.to_string())?;
    let (counter, data) = transport.encrypt(&encoded)?;
    send_host(writer, &TrustedHostFrame::Ciphertext { counter, data }).await
}

async fn run_command<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    transport: &mut Transport,
    program: &str,
    args: &[String],
    timeout_secs: u32,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(timeout_secs.into()), command.output())
        .await
        .map_err(|_| "approved root command timed out")?
        .map_err(|error| format!("spawn approved root command: {error}"))?;
    let mut stdout = output.stdout;
    let mut stderr = output.stderr;
    stdout.truncate(MAX_COMMAND_OUTPUT);
    stderr.truncate(MAX_COMMAND_OUTPUT.saturating_sub(stdout.len()));
    if !stdout.is_empty() {
        send_encrypted(
            writer,
            transport,
            TrustedPlaintext::Output {
                stream: "stdout".into(),
                data: stdout,
            },
        )
        .await?;
    }
    if !stderr.is_empty() {
        send_encrypted(
            writer,
            transport,
            TrustedPlaintext::Output {
                stream: "stderr".into(),
                data: stderr,
            },
        )
        .await?;
    }
    send_encrypted(
        writer,
        transport,
        TrustedPlaintext::Exit {
            code: output.status.code().unwrap_or(-1),
            message: "root command complete".into(),
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_pty<R, W>(
    reader: &mut R,
    writer: &mut W,
    transport: &mut Transport,
    shell: &str,
    ttl_secs: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let pair = NativePtySystem::default()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| error.to_string())?;
    let mut command = CommandBuilder::new(shell);
    command.arg("-l");
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| error.to_string())?;
    drop(pair.slave);
    let mut pty_reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| error.to_string())?;
    let mut pty_writer = pair
        .master
        .take_writer()
        .map_err(|error| error.to_string())?;
    let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match pty_reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    if output_tx.send(buffer[..length].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = tokio::time::sleep(Duration::from_secs(ttl_secs.into()));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            output = output_rx.recv() => {
                let Some(data) = output else { break };
                send_encrypted(writer, transport, TrustedPlaintext::Output {
                    stream: "pty".into(), data,
                }).await?;
            }
            incoming = framing::read_frame(reader) => {
                let payload = match incoming { Ok(value) => value, Err(_) => break };
                match shared::trusted::decode_client(&payload)? {
                    TrustedClientFrame::Ciphertext { counter, data } => {
                        let plaintext = transport.decrypt(counter, &data)?;
                        pty_writer.write_all(&plaintext).map_err(|error| error.to_string())?;
                        pty_writer.flush().map_err(|error| error.to_string())?;
                    }
                    TrustedClientFrame::Resize { cols, rows } => {
                        pair.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
                            .map_err(|error| error.to_string())?;
                    }
                    TrustedClientFrame::Close => break,
                    _ => return Err("unexpected frame in active root PTY".into()),
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    send_encrypted(
        writer,
        transport,
        TrustedPlaintext::Exit {
            code: 0,
            message: "root PTY closed".into(),
        },
    )
    .await
}
