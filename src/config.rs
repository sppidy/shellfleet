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
//! `/srv`, `/home`, `/var/log`). Reject any path that resolves into
//! the deny-list (the SSH dir, shadow files, the agent's own token,
//! kernel pseudo-FS, etc.) regardless of how it was spelled.
//!
//! Two layers: a fast lexical check (allow / deny prefix match on
//! the `..`-normalised string, [`check`]) and a symlink-resolving
//! re-check ([`check_read`] / [`check_write`]). Only the latter is
//! used by main.rs — `check` is the building block, kept `pub` so
//! the existing tests still cover the lexical layer.

use std::path::PathBuf;

const ALLOW_PREFIXES: &[&str] = &[
    "/etc/",
    "/opt/",
    "/usr/local/etc/",
    "/srv/",
    "/home/",
    "/var/log/",
    // `/tmp/` is intentionally NOT here. Any local user can place
    // a symlink in /tmp pointing at /etc/shadow or the agent's own
    // token; the canonicalize-and-recheck pass below catches
    // *resolved* targets, but it's cleaner to drop /tmp from the
    // allow-list entirely than to rely on every reviewer
    // remembering the symlink-trap reasoning.
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
    "/etc/shellfleet/agent-token",
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
    /// `canonicalize` failed to resolve the path. For reads this
    /// usually means the file doesn't exist; for writes (parent
    /// canonicalization) it means the parent dir doesn't exist.
    /// Surfaced as a distinct error so the operator sees the
    /// difference between "denied" and "not found".
    ResolveFailed(String),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::NotAbsolute => write!(f, "path must be absolute"),
            PathError::BlockedByDenyList(p) => write!(f, "path matches deny-list: {p}"),
            PathError::OutsideAllowList => write!(
                f,
                "path is outside the allowed config-file locations \
                 (/etc, /opt, /usr/local/etc, /srv, /home, /var/log)"
            ),
            PathError::InvalidUtf8 => write!(f, "path is not valid UTF-8"),
            PathError::Empty => write!(f, "path is empty"),
            PathError::ResolveFailed(e) => write!(f, "path resolve failed: {e}"),
        }
    }
}

/// Lexical-only check (allow / deny prefix match against the
/// `..`-normalised string). Used as the first stage of both
/// [`check_read`] and [`check_write`]. Callers should NOT use this
/// directly — it doesn't see through symlinks. Kept `pub` so the
/// existing tests still cover the lexical layer.
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
    // Symlink-based escapes are NOT caught here — that's why
    // main.rs goes through `check_read` / `check_write`, which
    // canonicalize and re-run the prefix match on the resolved
    // path / parent.
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

/// Validate a path for READ. Lexical check first, then resolve any
/// symlinks via `std::fs::canonicalize` and re-run the deny / allow
/// match against the resolved location. A symlink under an allowed
/// prefix that points at `/etc/shadow` (or anywhere outside the
/// allow-list) is rejected here.
pub fn check_read(path: &str) -> Result<PathBuf, PathError> {
    let lex = check(path)?;
    let canonical =
        std::fs::canonicalize(&lex).map_err(|e| PathError::ResolveFailed(e.to_string()))?;
    let canonical_str = canonical
        .to_str()
        .ok_or(PathError::InvalidUtf8)?
        .to_ascii_lowercase();
    for d in DENY_PREFIXES {
        if canonical_str.starts_with(d) {
            return Err(PathError::BlockedByDenyList(format!(
                "{d} (resolved from {path})"
            )));
        }
    }
    if !ALLOW_PREFIXES.iter().any(|a| canonical_str.starts_with(a)) {
        return Err(PathError::OutsideAllowList);
    }
    Ok(canonical)
}

/// Validate a path for WRITE. The target file may not exist yet
/// (creating a new operator config), so canonicalize the PARENT
/// dir instead and verify it's still inside the allow-list.
/// Returns `(canonical_parent, file_name)` — callers combine those
/// and open with [`write_no_follow`] so a symlink swap on the
/// final component still doesn't follow off-target.
pub fn check_write(path: &str) -> Result<(PathBuf, std::ffi::OsString), PathError> {
    let lex = check(path)?;
    let parent = lex.parent().ok_or(PathError::OutsideAllowList)?;
    let file_name = lex
        .file_name()
        .ok_or(PathError::OutsideAllowList)?
        .to_os_string();
    let canonical_parent =
        std::fs::canonicalize(parent).map_err(|e| PathError::ResolveFailed(e.to_string()))?;
    let mut canonical_str = canonical_parent
        .to_str()
        .ok_or(PathError::InvalidUtf8)?
        .to_ascii_lowercase();
    if !canonical_str.ends_with('/') {
        canonical_str.push('/');
    }
    for d in DENY_PREFIXES {
        if canonical_str.starts_with(d) {
            return Err(PathError::BlockedByDenyList(format!(
                "{d} (parent resolved from {path})"
            )));
        }
    }
    if !ALLOW_PREFIXES.iter().any(|a| canonical_str.starts_with(a)) {
        return Err(PathError::OutsideAllowList);
    }
    Ok((canonical_parent, file_name))
}

/// Open `parent/file_name` for writing with `O_NOFOLLOW`, refusing
/// to follow a symlink at the final component. Truncates / creates.
/// Combined with [`check_write`]'s canonicalised-parent guarantee,
/// this leaves only a tiny TOCTOU window where the parent dir
/// itself is replaced (which requires write access to the parent
/// of the parent — typically root-only on a sane host).
#[cfg(unix)]
pub fn write_no_follow(
    parent: &std::path::Path,
    file_name: &std::ffi::OsStr,
    content: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let target = parent.join(file_name);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&target)?;
    f.write_all(content)
}

/// Non-Unix fallback. The agent only ships on Linux today (the
/// `.deb` workflow + Helm chart Dockerfile both target glibc), but
/// `cargo build` on a developer's macOS / Windows host needs to
/// compile, so this stub lets the code build there. No
/// `O_NOFOLLOW` semantics are claimed.
#[cfg(not(unix))]
pub fn write_no_follow(
    parent: &std::path::Path,
    file_name: &std::ffi::OsStr,
    content: &[u8],
) -> std::io::Result<()> {
    let target = parent.join(file_name);
    std::fs::write(target, content)
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
            assert!(
                matches!(check(p), Err(PathError::BlockedByDenyList(_))),
                "should block {p}"
            );
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
            check("/etc/shellfleet/agent-token.txt"),
            Err(PathError::BlockedByDenyList(_))
        ));
    }

    #[test]
    fn blocks_proc_sys_dev() {
        for p in [
            "/proc/self/environ",
            "/sys/class/net/eth0/address",
            "/dev/sda",
        ] {
            assert!(matches!(check(p), Err(PathError::BlockedByDenyList(_))));
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
