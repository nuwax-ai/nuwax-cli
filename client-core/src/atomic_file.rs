//! Replace a file only after its complete contents have been written and synced.

use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum PermissionsPolicy {
    /// Sensitive state: Unix permissions are 0600, including during writing.
    Private,
    /// Retain an existing file's permissions; new files use tempfile's defaults.
    Preserve,
}

pub fn write_atomic(path: &Path, contents: &[u8], policy: PermissionsPolicy) -> Result<()> {
    write_atomic_with(path, policy, |file| file.write_all(contents))
}

fn write_atomic_with(
    path: &Path,
    policy: PermissionsPolicy,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<()> {
    // Match fs::write's existing follow-link behavior. Replacing the alias itself
    // would silently disconnect an .env managed by an external secrets file.
    let destination = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .with_context(|| format!("Failed to resolve file symlink: {}", path.display()))?,
        Ok(_) => path.to_path_buf(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect file path: {}", path.display()));
        }
    };
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create file directory: {}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temporary file for {}", path.display()))?;
    match policy {
        PermissionsPolicy::Private => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                temporary
                    .as_file()
                    .set_permissions(fs::Permissions::from_mode(0o600))?;
            }
        }
        PermissionsPolicy::Preserve => match fs::metadata(&destination) {
            Ok(metadata) => temporary
                .as_file()
                .set_permissions(metadata.permissions())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to read permissions of {}", path.display()));
            }
        },
    }
    write(temporary.as_file_mut())
        .with_context(|| format!("Failed to write temporary file for {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("Failed to sync temporary file for {}", path.display()))?;
    temporary
        .persist(&destination)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to replace {} atomically", path.display()))?;

    // persist performs an atomic replacement on Unix and Windows, but does not
    // sync directory metadata. Windows does not support this File operation.
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| {
            format!(
                "File {} was replaced, but syncing its parent directory failed",
                path.display()
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_write_failure_preserves_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        fs::write(&path, b"previous complete state").unwrap();
        let result = write_atomic_with(&path, PermissionsPolicy::Private, |file| {
            file.write_all(b"partial new state")?;
            Err(io::Error::other("injected disk write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"previous complete state");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn replaces_existing_file_and_creates_nested_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/state.json");
        write_atomic(&path, b"first", PermissionsPolicy::Private).unwrap();
        write_atomic(&path, b"second", PermissionsPolicy::Private).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }

    #[cfg(unix)]
    #[test]
    fn relative_symlink_is_retained_and_its_target_is_replaced() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let secrets = directory.path().join("secrets");
        fs::create_dir(&secrets).unwrap();
        let target = secrets.join("deployment.env");
        fs::write(&target, b"old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let alias = directory.path().join(".env");
        symlink("secrets/deployment.env", &alias).unwrap();
        write_atomic(&alias, b"new", PermissionsPolicy::Preserve).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read(&alias).unwrap(), b"new");
        assert_eq!(
            fs::read_link(&alias).unwrap(),
            Path::new("secrets/deployment.env")
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_and_alias_survive_partial_write_failure() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("original.env");
        fs::write(&target, b"complete previous content").unwrap();
        let alias = directory.path().join(".env");
        symlink("original.env", &alias).unwrap();
        let result = write_atomic_with(&alias, PermissionsPolicy::Preserve, |file| {
            file.write_all(b"partial replacement")?;
            Err(io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"complete previous content");
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new("original.env"));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlink_reports_error_without_replacing_alias() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let alias = directory.path().join(".env");
        symlink("missing.env", &alias).unwrap();
        assert!(write_atomic(&alias, b"new", PermissionsPolicy::Preserve).is_err());
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new("missing.env"));
        assert!(!directory.path().join("missing.env").exists());
    }

    #[cfg(unix)]
    #[test]
    fn private_permissions_and_preserved_env_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        write_atomic(&path, b"private", PermissionsPolicy::Private).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&path, b"env", PermissionsPolicy::Preserve).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        write_atomic(&path, b"private again", PermissionsPolicy::Private).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
