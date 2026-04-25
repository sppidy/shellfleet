use shared::AptUpgradable;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn last_apt_update_secs() -> u64 {
    // First successful apt-get update writes this stamp on Debian-derived
    // systems. Fall back to the package cache mtime — it's regenerated on
    // every apt-get update too.
    for path in [
        "/var/lib/apt/periodic/update-success-stamp",
        "/var/cache/apt/pkgcache.bin",
    ] {
        if let Ok(meta) = std::fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                if let Ok(d) = modified.duration_since(UNIX_EPOCH) {
                    return d.as_secs();
                }
            }
        }
    }
    0
}

fn parse_apt_list(stdout: &str) -> Vec<AptUpgradable> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        // Skip the leading "Listing... Done" header and blank lines.
        if line.is_empty() || line.starts_with("Listing") || line.starts_with("WARNING") {
            continue;
        }
        // Format: name/source new-version arch [upgradable from: old-version]
        // Example: sys-manager-agent/now 1.1.0-ci... amd64 [upgradable from: 1.0.0]
        let mut parts = line.split_whitespace();
        let name_source = match parts.next() {
            Some(s) => s,
            None => continue,
        };
        let new_version = match parts.next() {
            Some(s) => s,
            None => continue,
        };
        let _arch = parts.next();
        let mut current_version = String::new();
        let rest: Vec<&str> = parts.collect();
        let joined = rest.join(" ");
        if let Some(start) = joined.find("upgradable from: ") {
            let tail = &joined[start + "upgradable from: ".len()..];
            current_version = tail.trim_end_matches(']').trim().to_string();
        }
        let mut split = name_source.splitn(2, '/');
        let name = split.next().unwrap_or("").to_string();
        let source = split.next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        out.push(AptUpgradable {
            name,
            current_version,
            new_version: new_version.to_string(),
            source,
        });
    }
    out
}

pub async fn list_upgradable() -> Result<Vec<AptUpgradable>, String> {
    let output = Command::new("apt")
        .args(["list", "--upgradable"])
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("LC_ALL", "C")
        .output()
        .await
        .map_err(|e| format!("apt spawn: {e}"))?;
    // apt warns about CLI usage on stderr; that's fine, we only parse stdout.
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_apt_list(&stdout))
}

fn truncate_log(buf: &[u8], limit: usize) -> String {
    let text = String::from_utf8_lossy(buf);
    if text.len() <= limit {
        text.into_owned()
    } else {
        let cut = text.len() - limit;
        format!("…[{cut} bytes truncated]…\n{}", &text[text.len() - limit..])
    }
}

pub async fn refresh() -> (bool, String, Option<String>) {
    let output = match Command::new("apt-get")
        .arg("update")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("LC_ALL", "C")
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let success = output.status.success();
    let mut log = truncate_log(&output.stdout, 4000);
    if !output.stderr.is_empty() {
        log.push_str("\n--- stderr ---\n");
        log.push_str(&truncate_log(&output.stderr, 4000));
    }
    let _ = now_unix();
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", output.status.code()))
    };
    (success, log, err)
}

pub async fn upgrade(package: Option<String>) -> (bool, String, Option<String>) {
    let mut cmd = Command::new("apt-get");
    cmd.env("DEBIAN_FRONTEND", "noninteractive")
        .env("LC_ALL", "C")
        .arg("-y");
    match &package {
        Some(p) => {
            cmd.arg("install").arg("--only-upgrade").arg(p);
        }
        None => {
            cmd.arg("upgrade");
        }
    }
    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return (false, String::new(), Some(format!("spawn: {e}"))),
    };
    let success = output.status.success();
    let mut log = truncate_log(&output.stdout, 6000);
    if !output.stderr.is_empty() {
        log.push_str("\n--- stderr ---\n");
        log.push_str(&truncate_log(&output.stderr, 4000));
    }
    let err = if success {
        None
    } else {
        Some(format!("exit code {:?}", output.status.code()))
    };
    (success, log, err)
}
