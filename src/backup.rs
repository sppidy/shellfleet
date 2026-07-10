//! Filesystem backups: archive the requested paths into a destination
//! that's either a local path or an `s3://...` URI. v2 also supports
//! listing existing archives at a destination and restoring a named
//! archive to an operator-chosen root path.

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use shared::{BackupArchive, BackupMode};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

const LOG_CAP: usize = 8_000;
/// Above this many bytes we use S3 multipart upload (~5 MB minimum
/// per non-final part is the S3 rule). Single PUT under it is one
/// less round-trip and safer on small files.
const MULTIPART_THRESHOLD: u64 = 64 * 1024 * 1024;
const MULTIPART_CHUNK: usize = 16 * 1024 * 1024;
/// Hard ceiling for S3 GetObject body size during restore. The
/// streaming loop aborts past this point so a malicious or
/// runaway upload can't fill the agent's disk while tar happily
/// extracts. Override with `SHELLFLEET_S3_RESTORE_MAX_BYTES` (in
/// bytes); default is 50 GiB.
const S3_RESTORE_MAX_BYTES_DEFAULT: u64 = 50 * 1024 * 1024 * 1024;
/// Backups are confined to agent-managed state unless the operator makes an
/// explicit decision to allow unrestricted host access. This keeps a
/// compromised control plane from using the backup API to read or overwrite
/// host credentials by default.
const DEFAULT_BACKUP_ROOTS: &[&str] = &["/var/lib/shellfleet", "/var/backups/shellfleet"];

fn s3_restore_max_bytes() -> u64 {
    std::env::var("SHELLFLEET_S3_RESTORE_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(S3_RESTORE_MAX_BYTES_DEFAULT)
}

#[derive(Debug, Clone)]
pub enum Dest {
    Local(PathBuf),
    S3 { bucket: String, prefix: String },
}

pub fn parse_dest(dest: &str) -> Result<Dest, String> {
    let trimmed = dest.trim();
    if trimmed.is_empty() {
        return Err("dest is empty".into());
    }
    if let Some(rest) = trimmed.strip_prefix("s3://") {
        let mut parts = rest.splitn(2, '/');
        let bucket = parts.next().unwrap_or("").to_string();
        let prefix = parts.next().unwrap_or("").to_string();
        if bucket.is_empty() {
            return Err("s3 dest must include a bucket".into());
        }
        return Ok(Dest::S3 {
            bucket,
            prefix: prefix.trim_end_matches('/').to_string(),
        });
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        return Ok(Dest::Local(PathBuf::from(rest)));
    }
    if trimmed.contains("://") {
        return Err(format!(
            "unsupported destination scheme {trimmed:?} — supported: local path, file://…, s3://bucket/prefix"
        ));
    }
    Ok(Dest::Local(PathBuf::from(trimmed)))
}

/// Allow-list of host paths the server is permitted to back up (read) and
/// restore into (write). By default this is limited to agent-managed state.
/// Set `SHELLFLEET_BACKUP_ROOTS` to a colon-separated list of prefixes to
/// replace the defaults. Unrestricted host access requires the explicit
/// `SHELLFLEET_ALLOW_UNRESTRICTED_BACKUPS=1` opt-out.
fn backup_roots() -> Option<Vec<PathBuf>> {
    if std::env::var("SHELLFLEET_ALLOW_UNRESTRICTED_BACKUPS").as_deref() == Ok("1") {
        return None;
    }
    Some(configured_backup_roots(
        std::env::var("SHELLFLEET_BACKUP_ROOTS").ok().as_deref(),
    ))
}

fn configured_backup_roots(raw: Option<&str>) -> Vec<PathBuf> {
    raw.map(|raw| {
        raw.split(':')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    })
    .filter(|roots| !roots.is_empty())
    .unwrap_or_else(|| DEFAULT_BACKUP_ROOTS.iter().map(PathBuf::from).collect())
}

/// Lexically resolve `.`/`..` without touching the filesystem so a
/// `roots`-relative prefix check can't be defeated by `../` traversal.
fn normalize_lexical(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True iff `p` lives under one of `roots`. Requires a lexical prefix
/// match (kills `../` escapes); additionally, if `p` already exists, its
/// canonical (symlink-resolved) path must also stay under a root — so a
/// symlink inside an allowed dir can't point the operation outside it.
/// A not-yet-existing target (e.g. a fresh restore root) passes on the
/// lexical check alone, since its real ancestor will be created in-tree.
fn within_roots(p: &Path, roots: &[PathBuf]) -> bool {
    let lex = normalize_lexical(p);
    let lex_ok = roots.iter().any(|r| lex.starts_with(normalize_lexical(r)));
    if !lex_ok {
        return false;
    }
    if let Ok(canon) = p.canonicalize() {
        return roots.iter().any(|r| {
            let rc = r.canonicalize().unwrap_or_else(|_| normalize_lexical(r));
            canon.starts_with(&rc)
        });
    }
    true
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn truncate(s: &mut String, cap: usize) {
    if s.len() > cap {
        let cut = s.len() - cap;
        let head = format!("…[{cut} bytes truncated]…\n");
        s.drain(..cut);
        s.insert_str(0, &head);
    }
}

/// Build an S3 client from the standard AWS credential chain.
/// Honors AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_REGION /
/// AWS_ENDPOINT_URL (for MinIO / R2 / B2 / etc.) — same env shape
/// the previous `aws` CLI shellout used.
async fn s3_client() -> S3Client {
    let cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    S3Client::new(&cfg)
}

/// Single-PUT upload for files under MULTIPART_THRESHOLD.
async fn s3_put_object(
    client: &S3Client,
    bucket: &str,
    key: &str,
    path: &Path,
) -> Result<u64, String> {
    let body = ByteStream::from_path(path)
        .await
        .map_err(|e| format!("ByteStream::from_path: {e}"))?;
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("PutObject: {}", aws_sdk_err(&e)))?;
    Ok(bytes)
}

/// Multipart upload for larger files. Uses MULTIPART_CHUNK chunks;
/// aborts the upload on any part failure so we don't leave half-
/// finished multipart sessions accumulating storage on the bucket.
async fn s3_multipart_upload(
    client: &S3Client,
    bucket: &str,
    key: &str,
    path: &Path,
) -> Result<u64, String> {
    let create = client
        .create_multipart_upload()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| format!("CreateMultipartUpload: {}", aws_sdk_err(&e)))?;
    let upload_id = match create.upload_id() {
        Some(s) => s.to_string(),
        None => return Err("CreateMultipartUpload returned no upload_id".into()),
    };

    let result = upload_parts(client, bucket, key, &upload_id, path).await;
    match result {
        Ok((parts, total_bytes)) => {
            let completed = CompletedMultipartUpload::builder()
                .set_parts(Some(parts))
                .build();
            client
                .complete_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .multipart_upload(completed)
                .send()
                .await
                .map_err(|e| format!("CompleteMultipartUpload: {}", aws_sdk_err(&e)))?;
            Ok(total_bytes)
        }
        Err(e) => {
            // Best-effort abort; ignore secondary error.
            let _ = client
                .abort_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .send()
                .await;
            Err(e)
        }
    }
}

async fn upload_parts(
    client: &S3Client,
    bucket: &str,
    key: &str,
    upload_id: &str,
    path: &Path,
) -> Result<(Vec<CompletedPart>, u64), String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut buf = vec![0u8; MULTIPART_CHUNK];
    let mut parts: Vec<CompletedPart> = Vec::new();
    let mut part_number: i32 = 1;
    let mut total: u64 = 0;
    loop {
        // Drain a full chunk's worth (or the trailing remainder) from
        // the file. Tokio's read may not fill the buffer in one call.
        let mut filled = 0usize;
        loop {
            match file.read(&mut buf[filled..]).await {
                Ok(0) => break,
                Ok(n) => {
                    filled += n;
                    if filled == buf.len() {
                        break;
                    }
                }
                Err(e) => return Err(format!("read {}: {e}", path.display())),
            }
        }
        if filled == 0 {
            break;
        }
        total += filled as u64;
        let body = ByteStream::from(buf[..filled].to_vec());
        let resp = client
            .upload_part()
            .bucket(bucket)
            .key(key)
            .upload_id(upload_id)
            .part_number(part_number)
            .body(body)
            .send()
            .await
            .map_err(|e| format!("UploadPart#{part_number}: {}", aws_sdk_err(&e)))?;
        parts.push(
            CompletedPart::builder()
                .set_e_tag(resp.e_tag().map(|s| s.to_string()))
                .part_number(part_number)
                .build(),
        );
        part_number += 1;
    }
    Ok((parts, total))
}

/// Stringify an aws-sdk error in a way that surfaces the upstream
/// message (HTTP status, S3 code) without dragging in the entire
/// SdkError debug representation.
fn aws_sdk_err<E: std::fmt::Display>(e: &E) -> String {
    e.to_string()
}

/// Returns (success, archive_uri, bytes, log, error).
pub async fn run_backup(
    name: &str,
    paths: &[String],
    dest: &str,
    mode: BackupMode,
) -> (bool, String, u64, String, Option<String>) {
    if matches!(mode, BackupMode::Restic) {
        return (
            false,
            String::new(),
            0,
            String::new(),
            Some("restic mode is not implemented in this agent build".into()),
        );
    }
    let parsed = match parse_dest(dest) {
        Ok(p) => p,
        Err(e) => return (false, String::new(), 0, String::new(), Some(e)),
    };
    if paths.is_empty() {
        return (
            false,
            String::new(),
            0,
            String::new(),
            Some("paths is empty".into()),
        );
    }
    let mut log = String::new();
    let mut existing: Vec<String> = Vec::new();
    for p in paths {
        if Path::new(p).exists() {
            existing.push(p.clone());
        }
    }
    if existing.is_empty() {
        return (
            false,
            String::new(),
            0,
            log,
            Some("no requested paths exist on this host".into()),
        );
    }
    // Confine sources to the allow-list when one is configured.
    if let Some(roots) = backup_roots() {
        let bad: Vec<&str> = existing
            .iter()
            .filter(|p| !within_roots(Path::new(p), &roots))
            .map(|s| s.as_str())
            .collect();
        if !bad.is_empty() {
            return (
                false,
                String::new(),
                0,
                log,
                Some(format!(
                    "path(s) outside SHELLFLEET_BACKUP_ROOTS allow-list: {}",
                    bad.join(", ")
                )),
            );
        }
    }
    if existing.len() != paths.len() {
        log.push_str("WARN: skipping missing path(s): ");
        for p in paths {
            if !Path::new(p).exists() {
                log.push_str(p);
                log.push(' ');
            }
        }
        log.push('\n');
    }

    let archive_name = format!("{name}-{}.tar.gz", timestamp());
    match parsed {
        Dest::Local(dest_dir) => {
            if let Err(e) = std::fs::create_dir_all(&dest_dir) {
                return (
                    false,
                    String::new(),
                    0,
                    log,
                    Some(format!("mkdir {}: {e}", dest_dir.display())),
                );
            }
            let archive_path = dest_dir.join(&archive_name);
            let mut cmd = Command::new("tar");
            cmd.arg("--ignore-failed-read")
                .arg("-czf")
                .arg(&archive_path)
                .args(&existing)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let output = match cmd.output().await {
                Ok(o) => o,
                Err(e) => {
                    return (
                        false,
                        String::new(),
                        0,
                        log,
                        Some(format!("spawn tar: {e}")),
                    );
                }
            };
            if !output.stdout.is_empty() {
                log.push_str(&String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                log.push_str("--- stderr ---\n");
                log.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            truncate(&mut log, LOG_CAP);
            if !output.status.success() {
                return (
                    false,
                    String::new(),
                    0,
                    log,
                    Some(format!("tar exit {:?}", output.status.code())),
                );
            }
            let bytes = std::fs::metadata(&archive_path)
                .map(|m| m.len())
                .unwrap_or(0);
            log.push_str(&format!(
                "wrote {} ({} bytes)\n",
                archive_path.display(),
                bytes
            ));
            truncate(&mut log, LOG_CAP);
            (true, archive_path.display().to_string(), bytes, log, None)
        }
        Dest::S3 { bucket, prefix } => {
            // Two-phase: tar to a tempfile, then upload via aws-sdk-s3.
            // The previous CLI shellout streamed tar→aws so disk
            // overhead was zero, but `aws s3 cp` couldn't report bytes
            // back to us and didn't natively retry on connection drops.
            // The temp file pays one disk pass for accurate byte
            // tracking + native multipart with retry hooks.
            let key = if prefix.is_empty() {
                archive_name.clone()
            } else {
                format!("{prefix}/{archive_name}")
            };
            let s3_uri = format!("s3://{bucket}/{key}");

            let tmp = match tempfile::Builder::new()
                .prefix("shellfleet-backup-")
                .suffix(".tar.gz")
                .tempfile()
            {
                Ok(t) => t,
                Err(e) => {
                    return (false, String::new(), 0, log, Some(format!("tempfile: {e}")));
                }
            };
            let tmp_path = tmp.path().to_path_buf();

            let mut tar_cmd = Command::new("tar");
            tar_cmd
                .arg("--ignore-failed-read")
                .arg("-czf")
                .arg(&tmp_path)
                .args(&existing)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let tar_out = match tar_cmd.output().await {
                Ok(o) => o,
                Err(e) => {
                    return (
                        false,
                        String::new(),
                        0,
                        log,
                        Some(format!("spawn tar: {e}")),
                    );
                }
            };
            if !tar_out.stderr.is_empty() {
                log.push_str("--- tar stderr ---\n");
                log.push_str(&String::from_utf8_lossy(&tar_out.stderr));
            }
            truncate(&mut log, LOG_CAP);
            if !tar_out.status.success() {
                return (
                    false,
                    String::new(),
                    0,
                    log,
                    Some(format!("tar exit {:?}", tar_out.status.code())),
                );
            }
            let local_bytes = std::fs::metadata(&tmp_path).map(|m| m.len()).unwrap_or(0);

            let client = s3_client().await;
            let upload_result = if local_bytes >= MULTIPART_THRESHOLD {
                log.push_str(&format!(
                    "uploading {local_bytes} bytes via multipart ({}-byte chunks)\n",
                    MULTIPART_CHUNK
                ));
                s3_multipart_upload(&client, &bucket, &key, &tmp_path).await
            } else {
                log.push_str(&format!("uploading {local_bytes} bytes via PutObject\n"));
                s3_put_object(&client, &bucket, &key, &tmp_path).await
            };
            // Tempfile drops here either way (Drop unlinks /tmp file).
            drop(tmp);

            match upload_result {
                Ok(uploaded) => {
                    log.push_str(&format!("wrote {s3_uri} ({uploaded} bytes)\n"));
                    truncate(&mut log, LOG_CAP);
                    (true, s3_uri, uploaded, log, None)
                }
                Err(e) => {
                    truncate(&mut log, LOG_CAP);
                    (false, String::new(), 0, log, Some(e))
                }
            }
        }
    }
}

/// Enumerate `*.tar.gz` archives at the destination. Returns
/// (success, archives, error).
pub async fn list_archives(dest: &str) -> (bool, Vec<BackupArchive>, Option<String>) {
    let parsed = match parse_dest(dest) {
        Ok(p) => p,
        Err(e) => return (false, Vec::new(), Some(e)),
    };
    match parsed {
        Dest::Local(dir) => {
            if !dir.is_dir() {
                return (true, Vec::new(), None);
            }
            let mut out: Vec<BackupArchive> = Vec::new();
            let entries = match std::fs::read_dir(&dir) {
                Ok(it) => it,
                Err(e) => return (false, Vec::new(), Some(format!("read_dir: {e}"))),
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(".tar.gz"))
                    .unwrap_or(false)
                {
                    let meta = match std::fs::metadata(&path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    out.push(BackupArchive {
                        name: path.file_name().unwrap().to_string_lossy().into_owned(),
                        uri: path.display().to_string(),
                        bytes: meta.len(),
                        mtime,
                    });
                }
            }
            out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
            (true, out, None)
        }
        Dest::S3 { bucket, prefix } => {
            let client = s3_client().await;
            // Paginate through every page so a long-lived bucket
            // with > 1000 archives still surfaces them all.
            let mut continuation: Option<String> = None;
            let mut out: Vec<BackupArchive> = Vec::new();
            loop {
                let mut r = client.list_objects_v2().bucket(&bucket);
                if !prefix.is_empty() {
                    r = r.prefix(format!("{prefix}/"));
                }
                if let Some(t) = continuation.as_ref() {
                    r = r.continuation_token(t);
                }
                let page = match r.send().await {
                    Ok(p) => p,
                    Err(e) => {
                        return (
                            false,
                            Vec::new(),
                            Some(format!("ListObjectsV2: {}", aws_sdk_err(&e))),
                        );
                    }
                };
                for obj in page.contents() {
                    let key = match obj.key() {
                        Some(k) => k,
                        None => continue,
                    };
                    if !key.ends_with(".tar.gz") {
                        continue;
                    }
                    let name = key.rsplit('/').next().unwrap_or(key).to_string();
                    let bytes = obj.size().unwrap_or(0).max(0) as u64;
                    let mtime = obj.last_modified().map(|t| t.secs()).unwrap_or(0);
                    out.push(BackupArchive {
                        name,
                        uri: format!("s3://{bucket}/{key}"),
                        bytes,
                        mtime,
                    });
                }
                if !page.is_truncated().unwrap_or(false) {
                    break;
                }
                continuation = page.next_continuation_token().map(|s| s.to_string());
                if continuation.is_none() {
                    break;
                }
            }
            out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
            (true, out, None)
        }
    }
}

/// Restore a single archive (tar.gz) into `dest_root` on the agent.
/// Returns (success, log, error).
pub async fn restore(archive_uri: &str, dest_root: &str) -> (bool, String, Option<String>) {
    let dest_root_trim = dest_root.trim();
    if dest_root_trim.is_empty() {
        return (false, String::new(), Some("dest_root is empty".into()));
    }
    // A restore writes/overwrites files under dest_root, so it's the most
    // dangerous path the server controls. Confine it to the allow-list when set.
    if let Some(roots) = backup_roots() {
        if !within_roots(Path::new(dest_root_trim), &roots) {
            return (
                false,
                String::new(),
                Some(format!(
                    "restore dest_root {dest_root_trim:?} is outside the SHELLFLEET_BACKUP_ROOTS allow-list"
                )),
            );
        }
    }
    if let Err(e) = std::fs::create_dir_all(dest_root_trim) {
        return (
            false,
            String::new(),
            Some(format!("mkdir {dest_root_trim}: {e}")),
        );
    }
    let mut log = String::new();

    if let Some(rest) = archive_uri.strip_prefix("s3://") {
        // s3://bucket/key path. Stream GetObject body into tar's
        // stdin so we don't need 2x archive size on disk during
        // restore.
        let mut parts = rest.splitn(2, '/');
        let bucket = parts.next().unwrap_or("").to_string();
        let key = parts.next().unwrap_or("").to_string();
        if bucket.is_empty() || key.is_empty() {
            return (
                false,
                log,
                Some(format!("malformed archive URI: s3://{rest}")),
            );
        }

        let client = s3_client().await;
        let get = match client.get_object().bucket(&bucket).key(&key).send().await {
            Ok(o) => o,
            Err(e) => return (false, log, Some(format!("GetObject: {}", aws_sdk_err(&e)))),
        };

        let mut tar_cmd = Command::new("tar");
        tar_cmd
            .arg("-xzf")
            .arg("-")
            .arg("-C")
            .arg(dest_root_trim)
            // Hardening: never honor absolute member paths (an
            // archive could otherwise overwrite /etc/...), don't
            // try to chown to the recorded uid/gid (the agent runs
            // as root and would faithfully apply hostile values),
            // and don't carry over the recorded mode bits.
            .arg("--no-absolute-names")
            // Do not rely on GNU tar's implicit traversal stripping: reject
            // archive members that contain an explicit parent component.
            .arg("--exclude=../*")
            .arg("--exclude=*/../*")
            .arg("--no-same-owner")
            .arg("--no-same-permissions")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut tar = match tar_cmd.spawn() {
            Ok(c) => c,
            Err(e) => return (false, log, Some(format!("spawn tar: {e}"))),
        };
        let mut tar_stdin = match tar.stdin.take() {
            Some(s) => s,
            None => return (false, log, Some("tar stdin not piped".into())),
        };

        // Pump the GetObject body into tar's stdin. Always shut
        // down tar's stdin (success OR failure) so the wait_with_output
        // below doesn't hang waiting for an EOF that never comes; if
        // the body errored we kill tar explicitly and don't await it.
        let mut body = get.body;
        let mut pipe_err: Option<String> = None;
        let max_bytes = s3_restore_max_bytes();
        let mut total_bytes: u64 = 0;
        loop {
            match body.try_next().await {
                Ok(Some(chunk)) => {
                    total_bytes = total_bytes.saturating_add(chunk.len() as u64);
                    if total_bytes > max_bytes {
                        pipe_err = Some(format!(
                            "S3 GetObject body exceeded SHELLFLEET_S3_RESTORE_MAX_BYTES ({max_bytes} bytes); aborting restore"
                        ));
                        break;
                    }
                    if let Err(e) = tar_stdin.write_all(&chunk).await {
                        pipe_err = Some(format!("tar stdin write: {e}"));
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    pipe_err = Some(format!("S3 body read: {e}"));
                    break;
                }
            }
        }
        // Close stdin so tar can exit. Fine if shutdown errors after
        // a write error — we already have a primary failure to report.
        let _ = tar_stdin.shutdown().await;
        drop(tar_stdin);

        if let Some(e) = pipe_err {
            // Don't wait on tar — kill and skip wait so we don't hang
            // on a half-extracted archive.
            let _ = tar.kill().await;
            return (false, log, Some(e));
        }

        let tar_out = tar.wait_with_output().await;
        match tar_out {
            Ok(t) => {
                if !t.stderr.is_empty() {
                    log.push_str("--- tar stderr ---\n");
                    log.push_str(&String::from_utf8_lossy(&t.stderr));
                }
                truncate(&mut log, LOG_CAP);
                if !t.status.success() {
                    return (false, log, Some(format!("tar exit {:?}", t.status.code())));
                }
                log.push_str(&format!(
                    "restored s3://{bucket}/{key} into {dest_root_trim}\n"
                ));
                truncate(&mut log, LOG_CAP);
                (true, log, None)
            }
            Err(e) => (false, log, Some(format!("tar wait: {e}"))),
        }
    } else {
        // Local file path (with or without file:// prefix).
        let local = archive_uri.strip_prefix("file://").unwrap_or(archive_uri);
        if !Path::new(local).is_file() {
            return (false, log, Some(format!("archive not found: {local}")));
        }
        let mut cmd = Command::new("tar");
        cmd.arg("-xzf")
            .arg(local)
            .arg("-C")
            .arg(dest_root_trim)
            // See the s3 path above for the rationale on these
            // three flags. Same hardening for local archives.
            .arg("--no-absolute-names")
            .arg("--exclude=../*")
            .arg("--exclude=*/../*")
            .arg("--no-same-owner")
            .arg("--no-same-permissions")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = match cmd.output().await {
            Ok(o) => o,
            Err(e) => return (false, log, Some(format!("spawn tar: {e}"))),
        };
        if !output.stdout.is_empty() {
            log.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            log.push_str("--- tar stderr ---\n");
            log.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        truncate(&mut log, LOG_CAP);
        if !output.status.success() {
            return (
                false,
                log,
                Some(format!("tar exit {:?}", output.status.code())),
            );
        }
        log.push_str(&format!("restored {local} into {dest_root_trim}\n"));
        truncate(&mut log, LOG_CAP);
        (true, log, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_resolves_dotdot() {
        assert_eq!(
            normalize_lexical(Path::new("/srv/data/../etc")),
            PathBuf::from("/srv/etc")
        );
        assert_eq!(
            normalize_lexical(Path::new("/a/./b")),
            PathBuf::from("/a/b")
        );
    }

    #[test]
    fn within_roots_allows_inside() {
        let roots = vec![PathBuf::from("/srv/backups")];
        assert!(within_roots(Path::new("/srv/backups/db"), &roots));
        assert!(within_roots(Path::new("/srv/backups"), &roots));
    }

    #[test]
    fn within_roots_rejects_outside_and_traversal() {
        let roots = vec![PathBuf::from("/srv/backups")];
        assert!(!within_roots(Path::new("/etc/shadow"), &roots));
        // `..` escape out of an allowed root must be denied.
        assert!(!within_roots(Path::new("/srv/backups/../../etc"), &roots));
    }

    #[test]
    fn backup_roots_are_restrictive_without_configuration() {
        assert_eq!(
            configured_backup_roots(None),
            vec![
                PathBuf::from("/var/lib/shellfleet"),
                PathBuf::from("/var/backups/shellfleet"),
            ]
        );
    }

    #[test]
    fn parse_dest_round_trips() {
        assert!(matches!(parse_dest("/var/x"), Ok(Dest::Local(_))));
        assert!(matches!(parse_dest("file:///var/x"), Ok(Dest::Local(_))));
        assert!(matches!(
            parse_dest("s3://bucket/prefix"),
            Ok(Dest::S3 { .. })
        ));
        assert!(parse_dest("s3://").is_err());
        assert!(parse_dest("ftp://x").is_err());
    }
}
