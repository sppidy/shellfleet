//! Agent-side guard for the config-file Read/Write surface.
//!
//! Even with the server's RBAC middleware now restricting
//! `ReadConfigRequest` and `WriteConfigRequest` to admin sessions,
//! the agent must not blindly trust whatever path the server hands
//! it. An attacker who compromises the server, or an admin who
//! mistypes, should not be able to make the agent serve `/etc/shadow`,
//! the host's SSH host keys, or the agent's own token file over the
//! WebSocket.
//!
//! The policy: accept canonicalised absolute paths under one of a few
//! operator-config locations (`/etc`, `/opt`, `/usr/local/etc`,
//! `/srv`, `/home`, `/var/log`, `/tmp`). Reject any path that
//! resolves into the deny-list (the SSH dir, shadow files, the
//! agent's own token, kernel pseudo-FS, etc.) regardless of how it
//! was spelled.

use std::path::PathBuf;

const ALLOW_PREFIXES: &[&str] = &[
    "/etc/",
    "/opt/",
    "/usr/local/etc/",
    "/srv/",
    "/home/",
    "/var/log/",
    "/tmp/",
];

/// Specific files / dirs the agent must never read or write, even if
/// they happen to fall under an allowed prefix. Compared as a prefix
/// match against the canonicalised path, so a deny entry of
/// `/etc/shadow` also blocks `/etc/shadow-`, `/etc/shadow.old`, etc.
const DENY_PREFIXES: &[&str] = &[
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/sudoers",
    "/etc/ssh/",
    "/etc/sys-manager/agent-token",
    "/root/",
    "/home/.ssh/",
    "/proc/",
    "/sys/",
    "/dev/",
];

#[derive(Debug)]
pub enum PathError {
    NotAbsolute,
    BlockedByDenyList(String),
    OutsideAllowList,
    InvalidUtf8,
    Empty,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::NotAbsolute => write!(f, "path must be absolute"),
            PathError::BlockedByDenyList(p) => write!(f, "path matches deny-list: {p}"),
            PathError::OutsideAllowList => write!(
                f,
                "path is outside the allowed config-file locations \
                 (/etc, /opt, /usr/local/etc, /srv, /home, /var/log, /tmp)"
            ),
            PathError::InvalidUtf8 => write!(f, "path is not valid UTF-8"),
            PathError::Empty => write!(f, "path is empty"),
        }
    }
}

/// Validate (and partially normalise) a config-file path supplied by
/// the server. Returns the path the agent should actually open, or
/// an error. We do NOT call `canonicalize` in the WRITE case because
/// the target file may not exist yet; instead we lexically normalise
/// `..` segments and reject any that climb out of the allowed roots.
pub fn check(path: &str) -> Result<PathBuf, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    // Linux-style absolute path. We don't use `Path::is_absolute()`
    // because that follows the host OS convention (Windows: `C:\`),
    // and the agent's filesystem semantics are always POSIX —
    // including in CI that runs the unit tests on Windows.
    if !path.starts_with('/') {
        return Err(PathError::NotAbsolute);
    }
    if path.contains('\0') {
        return Err(PathError::InvalidUtf8);
    }

    // Lexical normalisation: drop `.`, resolve `..` without touching
    // the filesystem. We split manually instead of using
    // `Path::components` so that the parsing is identical across
    // host OSes — `Path::components` on Windows reinterprets POSIX
    // paths in ways the agent's tests would otherwise fail on. The
    // canonical filesystem the agent runs against is always Linux.
    //
    // Symlink-based escapes still leak through this — a symlink in
    // /tmp pointing at /etc/shadow would resolve at open() time. The
    // deny-list catches the high-value targets and the read/write
    // are also gated to admin role at the server.
    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if seg == ".." {
            // POSIX: /.. is /. Pop if we have a segment, otherwise stay
            // at root.
            stack.pop();
        } else {
            stack.push(seg);
        }
    }
    let trailing_slash = path.ends_with('/');
    let mut s = String::with_capacity(path.len());
    s.push('/');
    for (i, seg) in stack.iter().enumerate() {
        if i > 0 {
            s.push('/');
        }
        s.push_str(seg);
    }
    if trailing_slash && !stack.is_empty() {
        s.push('/');
    }
    let lower = s.to_ascii_lowercase();

    // Deny-list takes precedence over allow-list.
    for d in DENY_PREFIXES {
        if lower.starts_with(d) {
            return Err(PathError::BlockedByDenyList(d.to_string()));
        }
    }
    let allowed = ALLOW_PREFIXES.iter().any(|a| lower.starts_with(a));
    if !allowed {
        return Err(PathError::OutsideAllowList);
    }
    Ok(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative() {
        assert!(matches!(check("etc/passwd"), Err(PathError::NotAbsolute)));
    }

    #[test]
    fn rejects_dotdot_climb() {
        // /etc/../etc/shadow normalises to /etc/shadow (deny).
        assert!(matches!(
            check("/etc/../etc/shadow"),
            Err(PathError::BlockedByDenyList(_))
        ));
        // /etc/../../etc normalises to /etc (allow).
        assert!(check("/etc/../../etc/").is_ok());
    }

    #[test]
    fn blocks_shadow_family() {
        for p in [
            "/etc/shadow",
            "/etc/shadow-",
            "/etc/shadow.bak",
            "/etc/gshadow",
            "/etc/sudoers",
            "/etc/sudoers.d/x",
        ] {
            assert!(matches!(
                check(p),
                Err(PathError::BlockedByDenyList(_))
            ), "should block {p}");
        }
    }

    #[test]
    fn blocks_ssh() {
        assert!(matches!(
            check("/etc/ssh/sshd_config"),
            Err(PathError::BlockedByDenyList(_))
        ));
        assert!(matches!(
            check("/root/.ssh/authorized_keys"),
            Err(PathError::BlockedByDenyList(_))
        ));
    }

    #[test]
    fn blocks_agent_token() {
        assert!(matches!(
            check("/etc/sys-manager/agent-token.txt"),
            Err(PathError::BlockedByDenyList(_))
        ));
    }

    #[test]
    fn blocks_proc_sys_dev() {
        for p in ["/proc/self/environ", "/sys/class/net/eth0/address", "/dev/sda"] {
            assert!(matches!(
                check(p),
                Err(PathError::BlockedByDenyList(_))
            ));
        }
    }

    #[test]
    fn allows_typical_config_paths() {
        for p in [
            "/etc/nginx/nginx.conf",
            "/etc/hostname",
            "/opt/myapp/config.toml",
            "/usr/local/etc/foo",
            "/var/log/syslog",
            "/srv/data/notes.txt",
        ] {
            assert!(check(p).is_ok(), "should allow {p}");
        }
    }
}
