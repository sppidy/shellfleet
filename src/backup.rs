//! Filesystem backups: tar.gz the requested paths into a destination
//! directory on the agent host. v1 only supports a local destination
//! (no scheme, or `file://` scheme); other schemes are rejected with a
//! clear error so the operator sees the v1 limitation.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

const LOG_CAP: usize = 8_000;

fn parse_local_dest(dest: &str) -> Result<PathBuf, String> {
    let trimmed = dest.trim();
    if trimmed.is_empty() {
        return Err("dest is empty".into());
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        return Ok(PathBuf::from(rest));
    }
    if trimmed.contains("://") {
        return Err(format!(
            "unsupported destination scheme {trimmed:?} — v1 supports local paths or file://… only"
        ));
    }
    Ok(PathBuf::from(trimmed))
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // RFC 3339-ish, just for filenames. No external chrono dep needed
    // for the agent — secs since epoch is fine and unambiguous.
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

/// Returns (success, archive_path, bytes, log, error).
pub async fn run_backup(
    name: &str,
    paths: &[String],
    dest: &str,
) -> (bool, String, u64, String, Option<String>) {
    let dest_dir = match parse_local_dest(dest) {
        Ok(p) => p,
        Err(e) => return (false, String::new(), 0, String::new(), Some(e)),
    };
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        return (
            false,
            String::new(),
            0,
            String::new(),
            Some(format!("mkdir {}: {e}", dest_dir.display())),
        );
    }
    if paths.is_empty() {
        return (false, String::new(), 0, String::new(), Some("paths is empty".into()));
    }
    // Sanity: every path must exist. Don't error, just warn — tar will
    // skip missing entries with -W flag, but the operator should see it.
    let mut log = String::new();
    let mut missing = Vec::new();
    for p in paths {
        if !Path::new(p).exists() {
            missing.push(p.clone());
        }
    }
    if !missing.is_empty() {
        log.push_str(&format!(
            "WARN: skipping missing path(s): {}\n",
            missing.join(", ")
        ));
    }

    let archive_name = format!("{name}-{}.tar.gz", timestamp());
    let archive_path = dest_dir.join(&archive_name);

    let mut cmd = Command::new("tar");
    cmd.arg("--ignore-failed-read")
        .arg("-czf")
        .arg(&archive_path);
    for p in paths {
        if Path::new(p).exists() {
            cmd.arg(p);
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

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
