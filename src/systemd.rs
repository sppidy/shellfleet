use serde::Deserialize;
use shared::ServiceInfo;
use tokio::process::Command;

/// Cheap probe used at agent startup to decide whether to advertise the
/// `"systemd"` capability. `systemctl --version` exits 0 when the binary
/// + dbus are reachable; we don't need to parse the output.
pub async fn systemd_available() -> bool {
    Command::new("systemctl")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[derive(Debug, Deserialize)]
struct SystemctlJsonRow {
    unit: Option<String>,
    #[allow(dead_code)] // present in `systemctl --output=json`; parsed but not surfaced
    load: Option<String>,
    active: Option<String>,
    sub: Option<String>,
    description: Option<String>,
}

pub async fn list_services() -> Result<Vec<ServiceInfo>, String> {
    // Try the modern JSON output first — it's far less ambiguous than the
    // tabular form, which prefixes failed/dead units with a "●" bullet and
    // can wrap long descriptions across columns.
    let json = Command::new("systemctl")
        .args([
            "list-units",
            "--type=service",
            "--all",
            "--no-pager",
            "--output=json",
        ])
        .output()
        .await
        .map_err(|e| format!("systemctl spawn: {e}"))?;

    if json.status.success() {
        let stdout = String::from_utf8_lossy(&json.stdout);
        if let Ok(rows) = serde_json::from_str::<Vec<SystemctlJsonRow>>(&stdout) {
            return Ok(rows
                .into_iter()
                .filter_map(|r| {
                    Some(ServiceInfo {
                        name: r.unit?,
                        description: r.description.unwrap_or_default(),
                        status: r.sub.unwrap_or_default(),
                        active_state: r.active.unwrap_or_default(),
                    })
                })
                .collect());
        }
    }

    // Fall back to whitespace parsing for older systemd. This version skips
    // a leading "●" bullet that systemctl emits for non-loaded units, which
    // the earlier implementation used as the unit name and silently produced
    // garbage rows.
    let plain = Command::new("systemctl")
        .args([
            "list-units",
            "--type=service",
            "--all",
            "--no-pager",
            "--no-legend",
            "--plain",
        ])
        .output()
        .await
        .map_err(|e| format!("systemctl spawn: {e}"))?;

    if !plain.status.success() {
        return Err(String::from_utf8_lossy(&plain.stderr).into_owned());
    }

    let stdout = String::from_utf8_lossy(&plain.stdout);
    let mut services = Vec::new();
    for raw_line in stdout.lines() {
        let line = raw_line.trim_start();
        let line = line.trim_start_matches('●').trim_start();
        if line.is_empty() {
            continue;
        }
        // UNIT  LOAD  ACTIVE  SUB  DESCRIPTION
        let mut it = line.splitn(5, char::is_whitespace).filter(|s| !s.is_empty());
        let name = match it.next() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let _load = it.next().unwrap_or("");
        let active_state = it.next().unwrap_or("").to_string();
        let status = it.next().unwrap_or("").to_string();
        let description = it.next().unwrap_or("").trim().to_string();
        if !name.contains('.') {
            // first column wasn't actually a unit name (likely a header row
            // some systemd versions emit even with --no-legend)
            continue;
        }
        services.push(ServiceInfo {
            name,
            description,
            status,
            active_state,
        });
    }
    Ok(services)
}

/// True if `name` is a syntactically valid systemd unit name.
/// Restricts the alphabet so the value can't be misread by `systemctl`
/// as a flag (`--user`, `--version`) or otherwise expand into shell-
/// surprising tokens. systemd's own grammar is stricter than this
/// regex, but anything matching it is at minimum harmless to pass to
/// `systemctl` argv.
fn is_safe_unit_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('-') || name.len() > 256 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '\\'))
}

pub async fn control_service(name: &str, action: &str) -> Result<(), String> {
    let allowed_actions = ["start", "stop", "restart", "reload"];
    if !allowed_actions.contains(&action) {
        return Err(format!("invalid action: {action}"));
    }
    if !is_safe_unit_name(name) {
        return Err(format!("invalid unit name: {name}"));
    }

    let output = Command::new("systemctl")
        .args([action, name])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(())
}
