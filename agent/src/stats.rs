use shared::Message;
use tokio::process::Command;

/// Read a single space-separated number from /proc.
fn read_first_word(path: &str) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    Some(s.split_whitespace().next()?.to_string())
}

fn parse_meminfo() -> std::collections::HashMap<String, u64> {
    let s = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut out = std::collections::HashMap::new();
    for line in s.lines() {
        // Format: "MemTotal:       16321780 kB"
        let mut it = line.split(':');
        if let (Some(k), Some(rest)) = (it.next(), it.next()) {
            let v = rest.trim().split_whitespace().next().unwrap_or("0");
            if let Ok(n) = v.parse::<u64>() {
                out.insert(k.to_string(), n);
            }
        }
    }
    out
}

fn read_uptime() -> u64 {
    read_first_word("/proc/uptime")
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
}

fn read_loadavg() -> (f32, f32, f32) {
    let s = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut it = s.split_whitespace();
    let p = |s: Option<&str>| s.and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
    (p(it.next()), p(it.next()), p(it.next()))
}

fn read_cpu_count() -> u32 {
    // Count "processor" lines in /proc/cpuinfo. Falls back to 1.
    std::fs::read_to_string("/proc/cpuinfo")
        .map(|s| s.lines().filter(|l| l.starts_with("processor")).count() as u32)
        .ok()
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

fn read_kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn read_hostname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string())
}

async fn read_root_disk() -> (u64, u64) {
    // POSIX mode forces a stable 6-column layout:
    //   Filesystem  1024-blocks  Used  Available  Capacity  Mounted on
    // We deliberately don't combine -P with --output= because GNU df treats
    // them as conflicting: depending on the version the totals come back as
    // 0 or the call errors silently.
    let output = match Command::new("df").args(["-Pk", "/"]).output().await {
        Ok(o) if o.status.success() => o,
        _ => return (0, 0),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(row) = stdout.lines().nth(1) {
        let mut it = row.split_whitespace();
        let _filesystem = it.next();
        let total = it.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let used = it.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        return (total, used);
    }
    (0, 0)
}

pub async fn snapshot() -> Message {
    let mem = parse_meminfo();
    let mem_total_kb = mem.get("MemTotal").copied().unwrap_or(0);
    // Prefer MemAvailable (since kernel 3.14) — accounts for cache that can
    // be reclaimed under pressure. Fall back to MemFree on older kernels.
    let mem_available_kb = mem
        .get("MemAvailable")
        .copied()
        .or_else(|| mem.get("MemFree").copied())
        .unwrap_or(0);
    let swap_total_kb = mem.get("SwapTotal").copied().unwrap_or(0);
    let swap_free_kb = mem.get("SwapFree").copied().unwrap_or(0);

    let (load_1, load_5, load_15) = read_loadavg();
    let (root_total, root_used) = read_root_disk().await;

    Message::SystemStatsResponse {
        hostname: read_hostname(),
        kernel: read_kernel(),
        uptime_secs: read_uptime(),
        cpu_count: read_cpu_count(),
        load_1,
        load_5,
        load_15,
        mem_total_kb,
        mem_available_kb,
        swap_total_kb,
        swap_free_kb,
        root_disk_total_kb: root_total,
        root_disk_used_kb: root_used,
    }
}
