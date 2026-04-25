use serde::Deserialize;
use shared::ServiceInfo;
use tokio::process::Command;

#[derive(Debug, Deserialize)]
struct SystemctlJsonRow {
    unit: Option<String>,
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

pub async fn control_service(name: &str, action: &str) -> Result<(), String> {
    let allowed_actions = ["start", "stop", "restart", "reload"];
    if !allowed_actions.contains(&action) {
        return Err(format!("invalid action: {action}"));
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
