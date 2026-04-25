//! Filesystem backups: archive the requested paths into a destination
//! that's either a local path or an `s3://...` URI. v2 also supports
//! listing existing archives at a destination and restoring a named
//! archive to an operator-chosen root path.

use shared::{BackupArchive, BackupMode};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

const LOG_CAP: usize = 8_000;

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
        return Ok(Dest::S3 { bucket, prefix: prefix.trim_end_matches('/').to_string() });
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
        return (false, String::new(), 0, String::new(), Some("paths is empty".into()));
    }
    let mut log = String::new();
    let mut existing: Vec<String> = Vec::new();
    for p in paths {
        if Path::new(p).exists() {
            existing.push(p.clone());
        }
    }
    if existing.is_empty() {
        return (false, String::new(), 0, log, Some("no requested paths exist on this host".into()));
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
                    return (false, String::new(), 0, log, Some(format!("spawn tar: {e}")));
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
            let bytes = std::fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
            log.push_str(&format!("wrote {} ({} bytes)\n", archive_path.display(), bytes));
            truncate(&mut log, LOG_CAP);
            (true, archive_path.display().to_string(), bytes, log, None)
        }
        Dest::S3 { bucket, prefix } => {
            // Pipe `tar -czf -` into `aws s3 cp - s3://bucket/prefix/<name>`.
            // The aws CLI must be installed on the host and configured
            // (env vars or ~/.aws/credentials). For S3-compatible
            // backends, set AWS_ENDPOINT_URL on the agent service.
            let key = if prefix.is_empty() {
                archive_name.clone()
            } else {
                format!("{prefix}/{archive_name}")
            };
            let s3_uri = format!("s3://{bucket}/{key}");

            let mut tar_cmd = Command::new("tar");
            tar_cmd
                .arg("--ignore-failed-read")
                .arg("-czf")
                .arg("-")
                .args(&existing)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut tar = match tar_cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    return (false, String::new(), 0, log, Some(format!("spawn tar: {e}")));
                }
            };
            let tar_stdout = tar.stdout.take().expect("tar stdout piped");
            let tar_stdout_stdio: Stdio = match tar_stdout.try_into() {
                Ok(s) => s,
                Err(e) => {
                    return (false, String::new(), 0, log, Some(format!("pipe tar->aws: {e}")));
                }
            };

            let mut s3_cmd = Command::new("aws");
            s3_cmd
                .arg("s3")
                .arg("cp")
                .arg("-")
                .arg(&s3_uri)
                .stdin(tar_stdout_stdio)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let s3_output = s3_cmd.output().await;

            // Wait on tar separately so we can capture its stderr.
            let tar_out = tar.wait_with_output().await;

            match (tar_out, s3_output) {
                (Ok(t), Ok(s)) => {
                    if !t.stderr.is_empty() {
                        log.push_str("--- tar stderr ---\n");
                        log.push_str(&String::from_utf8_lossy(&t.stderr));
                    }
                    if !s.stdout.is_empty() {
                        log.push_str(&String::from_utf8_lossy(&s.stdout));
                    }
                    if !s.stderr.is_empty() {
                        log.push_str("--- aws stderr ---\n");
                        log.push_str(&String::from_utf8_lossy(&s.stderr));
                    }
                    truncate(&mut log, LOG_CAP);
                    if !t.status.success() {
                        return (
                            false,
                            String::new(),
                            0,
                            log,
                            Some(format!("tar exit {:?}", t.status.code())),
                        );
                    }
                    if !s.status.success() {
                        return (
                            false,
                            String::new(),
                            0,
                            log,
                            Some(format!("aws s3 cp exit {:?}", s.status.code())),
                        );
                    }
                    log.push_str(&format!("wrote {s3_uri}\n"));
                    truncate(&mut log, LOG_CAP);
                    // We don't easily get the byte count from `aws s3 cp`,
                    // so leave bytes=0 and let the operator inspect the
                    // bucket. (Listing later returns the size.)
                    (true, s3_uri, 0, log, None)
                }
                (Err(e), _) => (false, String::new(), 0, log, Some(format!("tar wait: {e}"))),
                (_, Err(e)) => (false, String::new(), 0, log, Some(format!("aws spawn: {e}"))),
            }
        }
    }
}

/// Enumerate `*.tar.gz` archives at the destination. Returns
/// (success, archives, error).
pub async fn list_archives(
    dest: &str,
) -> (bool, Vec<BackupArchive>, Option<String>) {
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
            let mut cmd = Command::new("aws");
            cmd.arg("s3api")
                .arg("list-objects-v2")
                .arg("--bucket")
                .arg(&bucket)
                .arg("--output")
                .arg("json");
            if !prefix.is_empty() {
                cmd.arg("--prefix").arg(format!("{prefix}/"));
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            let output = match cmd.output().await {
                Ok(o) => o,
                Err(e) => return (false, Vec::new(), Some(format!("aws spawn: {e}"))),
            };
            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr).into_owned();
                return (false, Vec::new(), Some(err));
            }
            #[derive(serde::Deserialize)]
            struct ListResp {
                #[serde(rename = "Contents", default)]
                contents: Vec<S3Object>,
            }
            #[derive(serde::Deserialize)]
            struct S3Object {
                #[serde(rename = "Key")]
                key: String,
                #[serde(rename = "Size", default)]
                size: u64,
                #[serde(rename = "LastModified", default)]
                last_modified: String,
            }
            let parsed: ListResp = match serde_json::from_slice(&output.stdout) {
                Ok(p) => p,
                Err(e) => {
                    // An empty bucket returns "{}" with no Contents key,
                    // which serde will tolerate via default. If we
                    // really fail to parse, bubble up.
                    return (false, Vec::new(), Some(format!("parse aws output: {e}")));
                }
            };
            let mut out: Vec<BackupArchive> = parsed
                .contents
                .into_iter()
                .filter(|o| o.key.ends_with(".tar.gz"))
                .map(|o| {
                    let name = o
                        .key
                        .rsplit('/')
                        .next()
                        .unwrap_or(&o.key)
                        .to_string();
                    let mtime = chrono_parse_iso(&o.last_modified);
                    BackupArchive {
                        name,
                        uri: format!("s3://{bucket}/{}", o.key),
                        bytes: o.size,
                        mtime,
                    }
                })
                .collect();
            out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
            (true, out, None)
        }
    }
}

fn chrono_parse_iso(s: &str) -> i64 {
    // S3 LastModified is RFC3339, e.g. "2026-04-25T16:34:12.000Z".
    // Avoid pulling in chrono just for this — parse the digits we need.
    // YYYY-MM-DDTHH:MM:SS
    if s.len() < 19 {
        return 0;
    }
    let bytes = s.as_bytes();
    let parse = |start: usize, end: usize| -> Option<i64> {
        std::str::from_utf8(&bytes[start..end]).ok()?.parse().ok()
    };
    let year = parse(0, 4).unwrap_or(0);
    let month = parse(5, 7).unwrap_or(0);
    let day = parse(8, 10).unwrap_or(0);
    let hour = parse(11, 13).unwrap_or(0);
    let minute = parse(14, 16).unwrap_or(0);
    let second = parse(17, 19).unwrap_or(0);
    // crude but adequate: convert to a sortable epoch-ish without
    // bringing in chrono. Days-from-epoch via a fixed table per month.
    if year < 1970 {
        return 0;
    }
    let days_per_month_leap = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let days_per_month_normal = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut days: i64 = 0;
    for y in 1970..year {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        days += if leap { 366 } else { 365 };
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let table = if leap { &days_per_month_leap } else { &days_per_month_normal };
    for m in 1..month {
        days += table[(m - 1) as usize];
    }
    days += day - 1;
    days * 86400 + hour * 3600 + minute * 60 + second
}

/// Restore a single archive (tar.gz) into `dest_root` on the agent.
/// Returns (success, log, error).
pub async fn restore(
    archive_uri: &str,
    dest_root: &str,
) -> (bool, String, Option<String>) {
    let dest_root_trim = dest_root.trim();
    if dest_root_trim.is_empty() {
        return (false, String::new(), Some("dest_root is empty".into()));
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
        // aws s3 cp <s3uri> - | tar -xzf - -C dest
        let s3_uri = format!("s3://{rest}");
        let mut s3_cmd = Command::new("aws");
        s3_cmd
            .arg("s3")
            .arg("cp")
            .arg(&s3_uri)
            .arg("-")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut s3 = match s3_cmd.spawn() {
            Ok(c) => c,
            Err(e) => return (false, log, Some(format!("spawn aws: {e}"))),
        };
        let s3_stdout = s3.stdout.take().expect("aws stdout piped");
        let s3_stdout_stdio: Stdio = match s3_stdout.try_into() {
            Ok(s) => s,
            Err(e) => return (false, log, Some(format!("pipe aws->tar: {e}"))),
        };

        let mut tar_cmd = Command::new("tar");
        tar_cmd
            .arg("-xzf")
            .arg("-")
            .arg("-C")
            .arg(dest_root_trim)
            .stdin(s3_stdout_stdio)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let tar_output = tar_cmd.output().await;
        let s3_output = s3.wait_with_output().await;

        match (s3_output, tar_output) {
            (Ok(s), Ok(t)) => {
                if !s.stderr.is_empty() {
                    log.push_str("--- aws stderr ---\n");
                    log.push_str(&String::from_utf8_lossy(&s.stderr));
                }
                if !t.stdout.is_empty() {
                    log.push_str(&String::from_utf8_lossy(&t.stdout));
                }
                if !t.stderr.is_empty() {
                    log.push_str("--- tar stderr ---\n");
                    log.push_str(&String::from_utf8_lossy(&t.stderr));
                }
                truncate(&mut log, LOG_CAP);
                if !s.status.success() {
                    return (false, log, Some(format!("aws exit {:?}", s.status.code())));
                }
                if !t.status.success() {
                    return (false, log, Some(format!("tar exit {:?}", t.status.code())));
                }
                log.push_str(&format!("restored {s3_uri} into {dest_root_trim}\n"));
                truncate(&mut log, LOG_CAP);
                (true, log, None)
            }
            (Err(e), _) => (false, log, Some(format!("aws wait: {e}"))),
            (_, Err(e)) => (false, log, Some(format!("tar wait: {e}"))),
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
            return (false, log, Some(format!("tar exit {:?}", output.status.code())));
        }
        log.push_str(&format!("restored {local} into {dest_root_trim}\n"));
        truncate(&mut log, LOG_CAP);
        (true, log, None)
    }
}
