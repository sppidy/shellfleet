use shared::AptUpgradable;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;

/// Where the agent journals an in-flight apt run. The actual dpkg
/// transaction runs in a transient systemd unit outside the agent
/// service cgroup, so a systemd/libc/agent self-upgrade can restart
/// this process without killing apt.
fn apt_state_path() -> PathBuf {
    PathBuf::from("/var/lib/sys-manager/apt-run.json")
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AptRunState {
    pub package: Option<String>,
    pub started_at: u64,
    pub log: String,
    /// Truncated to a sane limit on every flush.
    pub bytes_written: usize,
    /// Transient systemd unit that owns the apt/dpkg transaction.
    /// None means this state was written by an older agent.
    #[serde(default)]
    pub unit_name: Option<String>,
    /// Files written by the detached apt wrapper.
    #[serde(default)]
    pub log_path: Option<String>,
    #[serde(default)]
    pub result_path: Option<String>,
    #[serde(default)]
    pub script_path: Option<String>,
}

const APT_LOG_CAP: usize = 16_000;

fn write_state(state: &AptRunState) {
    if let Ok(json) = serde_json::to_string(state) {
        let path = apt_state_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, json);
    }
}

fn clear_state() {
    let _ = std::fs::remove_file(apt_state_path());
}

fn read_state() -> Option<AptRunState> {
    let path = apt_state_path();
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Read any persisted apt run state. The agent calls this once at
/// startup; if a run was in flight when we exited, we resume watching
/// the detached systemd unit instead of marking the upgrade failed.
pub fn pending_run() -> Option<AptRunState> {
    read_state()
}

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

fn state_file(name: &str) -> PathBuf {
    PathBuf::from("/var/lib/sys-manager").join(name)
}

fn validate_package_arg(package: &str) -> Result<(), String> {
    if package.is_empty() {
        return Err("empty package name".to_string());
    }
    if package.starts_with('-') {
        return Err("package name must not start with '-'".to_string());
    }
    if package
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-' | ':' | '~'))
    {
        Ok(())
    } else {
        Err("package name contains unsupported characters".to_string())
    }
}

fn write_upgrade_script(
    script_path: &PathBuf,
    log_path: &PathBuf,
    result_path: &PathBuf,
) -> Result<(), String> {
    let script = format!(
        r#"#!/bin/sh
set -u

MODE="$1"
PKG="${{2:-}}"
LOG_FILE="{log_path}"
RESULT_FILE="{result_path}"

export DEBIAN_FRONTEND=noninteractive
export LC_ALL=C

umask 077
mkdir -p "$(dirname "$LOG_FILE")" "$(dirname "$RESULT_FILE")"

{{
    echo "[sys-manager] apt upgrade unit started at $(date -Is)"
    echo "[sys-manager] mode=$MODE package=${{PKG:-<all>}}"

    if [ "$MODE" = "package" ]; then
        apt-get -y install --only-upgrade "$PKG"
    else
        apt-get -y upgrade
    fi
    rc=$?

    if [ "$rc" -ne 0 ]; then
        echo "[sys-manager] apt-get exited $rc; attempting automatic dpkg/apt recovery"
        dpkg --configure -a
        dpkg_rc=$?
        apt-get -y -f install
        fix_rc=$?
        if [ "$dpkg_rc" -eq 0 ] && [ "$fix_rc" -eq 0 ]; then
            rc=0
            echo "[sys-manager] automatic dpkg/apt recovery completed"
        else
            echo "[sys-manager] automatic recovery failed: dpkg=$dpkg_rc apt_fix=$fix_rc"
        fi
    fi

    if [ "$rc" -eq 0 ]; then
        printf 'success\n' > "$RESULT_FILE"
    else
        printf 'failed:%s\n' "$rc" > "$RESULT_FILE"
    fi
    echo "[sys-manager] apt upgrade unit finished with rc=$rc at $(date -Is)"
    exit "$rc"
}} > "$LOG_FILE" 2>&1
"#,
        log_path = log_path.display(),
        result_path = result_path.display()
    );
    if let Some(parent) = script_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create state dir: {e}"))?;
    }
    std::fs::write(script_path, script).map_err(|e| format!("write apt wrapper: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(script_path, perms)
            .map_err(|e| format!("chmod apt wrapper: {e}"))?;
    }
    Ok(())
}

fn read_capped_log(path: &str) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => Some(truncate_log(&bytes, APT_LOG_CAP)),
        Err(_) => None,
    }
}

fn cleanup_run_files(state: &AptRunState) {
    if let Some(path) = &state.log_path {
        let _ = std::fs::remove_file(path);
    }
    if let Some(path) = &state.script_path {
        let _ = std::fs::remove_file(path);
    }
    if let Some(path) = &state.result_path {
        let _ = std::fs::remove_file(path);
    }
}

async fn systemctl_unit_state(unit: &str) -> Option<(String, String, String)> {
    let output = Command::new("systemctl")
        .args([
            "show",
            unit,
            "--property=LoadState",
            "--property=ActiveState",
            "--property=Result",
            "--no-page",
        ])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut load = String::new();
    let mut active = String::new();
    let mut result = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("LoadState=") {
            load = v.to_string();
        } else if let Some(v) = line.strip_prefix("ActiveState=") {
            active = v.to_string();
        } else if let Some(v) = line.strip_prefix("Result=") {
            result = v.to_string();
        }
    }
    Some((load, active, result))
}

async fn watch_systemd_run(mut state: AptRunState) -> (bool, String, Option<String>) {
    let result_path = match state.result_path.clone() {
        Some(p) => p,
        None => {
            let log = if state.log.is_empty() {
                "[sys-manager] older agent left an apt run marker without a detached unit\n"
                    .to_string()
            } else {
                state.log.clone()
            };
            clear_state();
            return (
                false,
                log,
                Some("older agent interrupted apt before detached execution was available".into()),
            );
        }
    };

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;

        if let Some(log_path) = &state.log_path
            && let Some(log) = read_capped_log(log_path)
        {
            state.log = log;
            state.bytes_written = state.log.len();
            write_state(&state);
        }

        if let Ok(result) = std::fs::read_to_string(&result_path) {
            let trimmed = result.trim();
            let success = trimmed == "success";
            let error = if success {
                None
            } else {
                Some(format!("apt unit {}", trimmed))
            };
            if state.log.is_empty() {
                state
                    .log
                    .push_str("[sys-manager] apt unit completed with no captured log\n");
            }
            cleanup_run_files(&state);
            clear_state();
            return (success, state.log, error);
        }

        if let Some(unit) = &state.unit_name {
            if let Some((load, active, result)) = systemctl_unit_state(unit).await {
                if load == "not-found" || (active == "failed" && result != "success") {
                    let mut log = state.log.clone();
                    log.push_str(&format!(
                        "\n[sys-manager] apt transient unit ended unexpectedly: load={load} active={active} result={result}\n"
                    ));
                    cleanup_run_files(&state);
                    clear_state();
                    return (
                        false,
                        log,
                        Some("apt transient unit ended before writing a result".into()),
                    );
                }
            }
        }
    }
}

async fn start_systemd_run(package: Option<String>) -> Result<AptRunState, String> {
    if let Some(pkg) = &package {
        validate_package_arg(pkg)?;
    }

    let started_at = now_unix();
    let pid = std::process::id();
    let run_id = format!("{started_at}-{pid}");
    let unit_name = format!("sys-manager-apt-{run_id}.service");
    let log_path = state_file(&format!("apt-run-{run_id}.log"));
    let result_path = state_file(&format!("apt-run-{run_id}.result"));
    let script_path = state_file(&format!("apt-run-{run_id}.sh"));

    write_upgrade_script(&script_path, &log_path, &result_path)?;

    let mut state = AptRunState {
        package: package.clone(),
        started_at,
        log: format!("[sys-manager] starting apt in detached systemd unit {unit_name}\n"),
        bytes_written: 0,
        unit_name: Some(unit_name.clone()),
        log_path: Some(log_path.display().to_string()),
        result_path: Some(result_path.display().to_string()),
        script_path: Some(script_path.display().to_string()),
    };
    state.bytes_written = state.log.len();
    write_state(&state);

    let mode = if package.is_some() { "package" } else { "all" };
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--unit")
        .arg(&unit_name)
        .arg("--description")
        .arg("sys-manager apt upgrade")
        .arg("--property=Restart=no")
        .arg("--property=TimeoutStartSec=0")
        .arg("--no-block")
        .arg("/bin/sh")
        .arg(&script_path)
        .arg(mode);
    if let Some(pkg) = &package {
        cmd.arg(pkg);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("systemd-run spawn: {e}"))?;
    if !output.status.success() {
        let mut err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            err = String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
        cleanup_run_files(&state);
        clear_state();
        return Err(format!("systemd-run failed: {err}"));
    }

    Ok(state)
}

pub async fn upgrade(package: Option<String>) -> (bool, String, Option<String>) {
    if let Some(existing) = read_state() {
        return watch_systemd_run(existing).await;
    }

    let state = match start_systemd_run(package).await {
        Ok(s) => s,
        Err(e) => return (false, String::new(), Some(e)),
    };
    watch_systemd_run(state).await
}

pub async fn resume_pending_upgrade() -> Option<(Option<String>, bool, String, Option<String>)> {
    let state = pending_run()?;
    let package = state.package.clone();
    let (success, log, error) = watch_systemd_run(state).await;
    Some((package, success, log, error))
}

#[cfg(test)]
mod tests {
    use super::{AptRunState, validate_package_arg, write_upgrade_script};

    #[test]
    fn accepts_debian_package_name_characters() {
        assert!(validate_package_arg("linux-image-6.8.0-40-generic").is_ok());
        assert!(validate_package_arg("libc6:amd64").is_ok());
        assert!(validate_package_arg("sys-manager-agent").is_ok());
    }

    #[test]
    fn rejects_option_or_shell_like_package_names() {
        assert!(validate_package_arg("-oDpkg::Options::=--force").is_err());
        assert!(validate_package_arg("pkg;reboot").is_err());
        assert!(validate_package_arg("").is_err());
    }

    #[test]
    fn generated_wrapper_runs_automatic_dpkg_recovery() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = tmp.path().join("apt-run.sh");
        let log = tmp.path().join("apt-run.log");
        let result = tmp.path().join("apt-run.result");

        write_upgrade_script(&script, &log, &result).expect("write wrapper");
        let text = std::fs::read_to_string(&script).expect("read wrapper");

        assert!(text.contains("apt-get -y upgrade"));
        assert!(text.contains("apt-get -y install --only-upgrade"));
        assert!(text.contains("dpkg --configure -a"));
        assert!(text.contains("apt-get -y -f install"));
        assert!(text.contains("printf 'success\\n'"));
        assert!(text.contains("printf 'failed:%s\\n'"));
    }

    #[test]
    fn old_state_json_defaults_detached_unit_fields() {
        let json = r#"{
            "package": "systemd",
            "started_at": 1777900000,
            "log": "old agent state",
            "bytes_written": 15
        }"#;

        let state: AptRunState = serde_json::from_str(json).expect("deserialize old state");
        assert_eq!(state.package.as_deref(), Some("systemd"));
        assert!(state.unit_name.is_none());
        assert!(state.log_path.is_none());
        assert!(state.result_path.is_none());
        assert!(state.script_path.is_none());
    }
}
