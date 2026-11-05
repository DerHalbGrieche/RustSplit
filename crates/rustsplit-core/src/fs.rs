//! Crash-safe file writes.
//!
//! Files are never written in place.  The new contents go to a temporary file
//! in the target's directory, are flushed to disk and only then renamed over
//! the target, so a crash, power loss or full disk leaves either the old or
//! the new file behind, never a truncated one.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Atomically replaces `path` with `contents`.
pub fn write_atomic(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_with(path.as_ref(), false, |file| {
        file.write_all(contents.as_ref())
    })
}

/// Like [`write_atomic`], but first keeps the previous version of `path` in
/// [`backup_path`].  The backup is only updated once the new contents are
/// safely in place, so a failed save never touches it.
pub fn write_atomic_with_backup(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> io::Result<()> {
    write_with(path.as_ref(), true, |file| {
        file.write_all(contents.as_ref())
    })
}

/// The backup location used by [`write_atomic_with_backup`], e.g.
/// `MyRun.lss.bak` for `MyRun.lss`.
pub fn backup_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(".bak");
    path.with_file_name(name)
}

fn write_with(
    path: &Path,
    backup: bool,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    // Write through symlinks instead of replacing them with a regular file.
    let path = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let existing = std::fs::metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file());

    let (temporary, mut file) = create_temporary(&path)?;
    let result = (|| {
        write(&mut file)?;
        if let Some(metadata) = &existing {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    // Copy the old version aside before replacing it, but only move the copy
    // into place after the rename succeeded.
    let staged_backup = if backup && existing.is_some() {
        match stage_backup(&path) {
            Ok(staged) => Some(staged),
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        }
    } else {
        None
    };

    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        if let Some(staged) = staged_backup {
            let _ = std::fs::remove_file(staged);
        }
        return Err(error);
    }
    // The save itself succeeded at this point, so a failure to move the backup
    // into place must not be reported as a failed save.
    if let Some(staged) = staged_backup {
        if std::fs::rename(&staged, backup_path(&path)).is_err() {
            let _ = std::fs::remove_file(staged);
        }
    }
    sync_parent(&path);
    Ok(())
}

fn stage_backup(path: &Path) -> io::Result<PathBuf> {
    let (staged, mut file) = create_temporary(&backup_path(path))?;
    let result = (|| {
        io::copy(&mut File::open(path)?, &mut file)?;
        file.set_permissions(std::fs::metadata(path)?.permissions())?;
        file.sync_all()
    })();
    drop(file);
    match result {
        Ok(()) => Ok(staged),
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            Err(error)
        }
    }
}

/// Creates a new, uniquely named hidden file next to `path`.
fn create_temporary(path: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    loop {
        let mut temporary_name = OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = path.with_file_name(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Makes the rename itself durable.  This is best effort: the new contents
/// are already in place and some filesystems don't support syncing
/// directories.
#[cfg(unix)]
fn sync_parent(path: &Path) {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("rustsplit-fs-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn entries(&self) -> Vec<String> {
            let mut entries: Vec<_> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            entries.sort();
            entries
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_and_replaces_without_leftovers() {
        let dir = TestDir::new("replace");
        let path = dir.0.join("settings.toml");
        write_atomic(&path, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        write_atomic(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(dir.entries(), ["settings.toml"]);
    }

    #[test]
    fn backup_holds_previous_version() {
        let dir = TestDir::new("backup");
        let path = dir.0.join("Run.lss");
        write_atomic_with_backup(&path, "one").unwrap();
        assert_eq!(dir.entries(), ["Run.lss"]);
        write_atomic_with_backup(&path, "two").unwrap();
        write_atomic_with_backup(&path, "three").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "three");
        assert_eq!(std::fs::read_to_string(backup_path(&path)).unwrap(), "two");
        assert_eq!(dir.entries(), ["Run.lss", "Run.lss.bak"]);
    }

    #[test]
    fn failed_write_leaves_original_and_backup_untouched() {
        let dir = TestDir::new("failure");
        let path = dir.0.join("Run.lss");
        write_atomic_with_backup(&path, "old").unwrap();
        write_atomic_with_backup(&path, "current").unwrap();
        let error = write_with(&path, true, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::new(io::ErrorKind::StorageFull, "disk full"))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "current");
        assert_eq!(std::fs::read_to_string(backup_path(&path)).unwrap(), "old");
        assert_eq!(dir.entries(), ["Run.lss", "Run.lss.bak"]);
    }

    #[test]
    fn missing_directory_fails_cleanly() {
        let dir = TestDir::new("missing");
        let path = dir.0.join("missing").join("Run.lss");
        assert!(write_atomic_with_backup(&path, "data").is_err());
        assert!(dir.entries().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("permissions");
        let path = dir.0.join("Run.lss");
        write_atomic(&path, "one").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic_with_backup(&path, "two").unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&backup_path(&path)), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn writes_through_symlinks() {
        let dir = TestDir::new("symlink");
        let target = dir.0.join("Real.lss");
        let link = dir.0.join("Link.lss");
        write_atomic(&target, "one").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_atomic(&link, "two").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "two");
    }
}
