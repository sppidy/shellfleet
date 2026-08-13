use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_STATE_DIR: &str = "/var/lib/shellfleet-agent";
const ACCESS_TOKEN_FILE: &str = "agent-token.txt";
const REFRESH_TOKEN_FILE: &str = "agent-refresh.txt";
const TOKEN_EXPIRY_FILE: &str = "agent-token-expiry.txt";
const MAX_CREDENTIAL_BYTES: u64 = 16 * 1024;

/// The agent's on-disk credential set.
///
/// Access-token replacement is the commit point for a rotation. Refresh
/// material is written first so an interrupted rotation leaves either the old
/// usable access token plus the new refresh token, or the complete new set.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    state_dir: PathBuf,
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new(DEFAULT_STATE_DIR)
    }
}

impl CredentialStore {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    fn path(&self, file: &str) -> PathBuf {
        self.state_dir.join(file)
    }

    pub fn access_token(&self) -> io::Result<Option<String>> {
        read_trimmed(&self.path(ACCESS_TOKEN_FILE))
    }

    pub fn refresh_token(&self) -> io::Result<Option<String>> {
        read_trimmed(&self.path(REFRESH_TOKEN_FILE))
    }

    pub fn token_expiry(&self) -> io::Result<Option<i64>> {
        let Some(raw) = read_trimmed(&self.path(TOKEN_EXPIRY_FILE))? else {
            return Ok(None);
        };
        raw.parse::<i64>().map(Some).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid agent token expiry: {error}"),
            )
        })
    }

    /// Persist one credential generation in recovery-safe order.
    pub fn persist(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<i64>,
    ) -> io::Result<()> {
        if access_token.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "access token must not be empty",
            ));
        }
        if access_token.len() as u64 > MAX_CREDENTIAL_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "access token exceeds credential size limit",
            ));
        }
        if refresh_token.is_some_and(|value| value.len() as u64 > MAX_CREDENTIAL_BYTES) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refresh token exceeds credential size limit",
            ));
        }

        replace_optional(
            &self.path(REFRESH_TOKEN_FILE),
            refresh_token.filter(|value| !value.trim().is_empty()),
        )?;
        let expiry = expires_at.map(|value| value.to_string());
        replace_optional(&self.path(TOKEN_EXPIRY_FILE), expiry.as_deref())?;

        // Commit marker: once this rename lands, its matching refresh material
        // is already durable.
        atomic_write(&self.path(ACCESS_TOKEN_FILE), access_token)
    }
}

fn replace_optional(path: &Path, contents: Option<&str>) -> io::Result<()> {
    match contents {
        Some(contents) => atomic_write(path, contents),
        None => match std::fs::remove_file(path) {
            Ok(()) => sync_parent(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    }
}

fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "credential has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o600))?;
    }

    temporary.write_all(contents.as_bytes())?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "credential has no parent"))?;
    std::fs::File::open(parent)?.sync_all()
}

fn read_trimmed(path: &Path) -> io::Result<Option<String>> {
    let file = match open_secret(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.file_type().is_file() {
        return Err(io::Error::other("credential path is not a regular file"));
    }

    let mut bytes = Vec::new();
    file.take(MAX_CREDENTIAL_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("credential exceeds {MAX_CREDENTIAL_BYTES}-byte limit"),
        ));
    }
    let value = String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let value = value.trim();
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

#[cfg(unix)]
fn open_secret(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_secret(path: &Path) -> io::Result<std::fs::File> {
    std::fs::File::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_replaces_a_complete_credential_set() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(directory.path());
        store
            .persist("access-one", Some("refresh-one"), Some(123))
            .unwrap();
        store
            .persist("access-two", Some("refresh-two"), Some(456))
            .unwrap();

        assert_eq!(store.access_token().unwrap().as_deref(), Some("access-two"));
        assert_eq!(
            store.refresh_token().unwrap().as_deref(),
            Some("refresh-two")
        );
        assert_eq!(store.token_expiry().unwrap(), Some(456));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for file in [ACCESS_TOKEN_FILE, REFRESH_TOKEN_FILE, TOKEN_EXPIRY_FILE] {
                let mode = std::fs::metadata(directory.path().join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "unexpected mode for {file}");
            }
        }
    }

    #[test]
    fn legacy_generation_removes_stale_rotation_material() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(directory.path());
        store
            .persist("access-one", Some("refresh-one"), Some(123))
            .unwrap();
        store.persist("legacy-access", None, None).unwrap();

        assert_eq!(
            store.access_token().unwrap().as_deref(),
            Some("legacy-access")
        );
        assert_eq!(store.refresh_token().unwrap(), None);
        assert_eq!(store.token_expiry().unwrap(), None);
    }

    #[test]
    fn oversized_credentials_are_rejected_instead_of_truncated() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join(ACCESS_TOKEN_FILE),
            vec![b'x'; MAX_CREDENTIAL_BYTES as usize + 1],
        )
        .unwrap();
        let store = CredentialStore::new(directory.path());

        let error = store.access_token().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn oversized_credentials_are_rejected_before_persistence() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(directory.path());
        let oversized = "x".repeat(MAX_CREDENTIAL_BYTES as usize + 1);

        let error = store.persist(&oversized, None, None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!directory.path().join(ACCESS_TOKEN_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn reads_reject_symlinks_and_writes_replace_them_without_following() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let victim = directory.path().join("victim");
        let access = directory.path().join(ACCESS_TOKEN_FILE);
        std::fs::write(&victim, "do-not-touch").unwrap();
        symlink(&victim, &access).unwrap();
        let store = CredentialStore::new(directory.path());

        assert!(store.access_token().is_err());
        store.persist("safe-access", None, None).unwrap();

        assert_eq!(std::fs::read_to_string(victim).unwrap(), "do-not-touch");
        assert_eq!(
            store.access_token().unwrap().as_deref(),
            Some("safe-access")
        );
        assert!(
            std::fs::symlink_metadata(access)
                .unwrap()
                .file_type()
                .is_file()
        );
    }
}
