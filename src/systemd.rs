use shared::ServiceInfo;
use tokio::process::Command;

pub async fn list_services() -> Result<Vec<ServiceInfo>, String> {
    let output = Command::new("systemctl")
        .args(&["list-units", "--type=service", "--all", "--no-pager", "--no-legend"])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut services = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // systemctl output format usually:
        // UNIT LOAD ACTIVE SUB DESCRIPTION
        if parts.len() >= 4 {
            let name = parts[0].to_string();
            let _load = parts[1];
            let active_state = parts[2].to_string();
            let status = parts[3].to_string();
            let description = parts[4..].join(" ");
            
            services.push(ServiceInfo {
                name,
                description,
                status,
                active_state,
            });
        }
    }

    Ok(services)
}

pub async fn control_service(name: &str, action: &str) -> Result<(), String> {
    let allowed_actions = ["start", "stop", "restart"];
    if !allowed_actions.contains(&action) {
        return Err("Invalid action".to_string());
    }

    let output = Command::new("systemctl")
        .args(&[action, name])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    Ok(())
}
