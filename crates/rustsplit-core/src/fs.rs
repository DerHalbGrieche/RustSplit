//! Crash-safe file writes.
//!
//! Files are never written in place.  The new contents go to a temporary file
//! in the target's directory, are flushed to disk and only then renamed over
//! the target, so a crash, power loss or full disk leaves either the old or
//! the new file behind, never a truncated one.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, Permissions};
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
        Ok(metadata) if metadata.file_type().is_symlink() => resolve_symlink(path)?,
        _ => path.to_path_buf(),
    };
    let existing = std::fs::metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file());
    // Writing in place refused read-only files, so the rename must not
    // quietly replace one either.
    if existing
        .as_ref()
        .is_some_and(|metadata| metadata.permissions().readonly())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is read-only", path.display()),
        ));
    }
    let permissions = existing.as_ref().map(|metadata| metadata.permissions());

    let (temporary, mut file) = create_temporary(&path, permissions.as_ref())?;
    let result = (|| {
        write(&mut file)?;
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

    if let Err(error) = rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        if let Some(staged) = staged_backup {
            let _ = std::fs::remove_file(staged);
        }
        return Err(error);
    }
    // The save itself succeeded at this point, so a failure to move the backup
    // into place must not be reported as a failed save.
    if let Some(staged) = staged_backup {
        if rename(&staged, &backup_path(&path)).is_err() {
            let _ = std::fs::remove_file(staged);
        }
    }
    sync_parent(&path);
    Ok(())
}

/// The file a symlink finally points to.  Unlike `canonicalize` this also
/// works when that file does not exist yet, so a dangling link gets its
/// target created, like writing through the link in place would.
fn resolve_symlink(link: &Path) -> io::Result<PathBuf> {
    match std::fs::canonicalize(link) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        result => return result,
    }
    // Follow the chain by hand, with Linux's limit so a loop cannot spin
    // forever.
    let mut path = link.to_path_buf();
    for _ in 0..40 {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {}
            _ => return Ok(path),
        }
        let target = std::fs::read_link(&path)?;
        // Relative targets are relative to the directory holding the link.
        path = match path.parent() {
            Some(parent) if target.is_relative() => parent.join(target),
            _ => target,
        };
    }
    Err(io::Error::other(format!(
        "too many levels of symbolic links at {}",
        link.display()
    )))
}

fn stage_backup(path: &Path) -> io::Result<PathBuf> {
    let permissions = std::fs::metadata(path)?.permissions();
    let (staged, mut file) = create_temporary(&backup_path(path), Some(&permissions))?;
    let result = (|| {
        io::copy(&mut File::open(path)?, &mut file)?;
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

/// Creates a new, uniquely named hidden file next to `path`, with
/// `permissions` applied before anything is written to it.
fn create_temporary(path: &Path, permissions: Option<&Permissions>) -> io::Result<(PathBuf, File)> {
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
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        // Create the file with the target's mode (the umask can only remove
        // bits), so a private file is never readable by others, not even
        // before the exact mode is applied below.
        #[cfg(unix)]
        if let Some(permissions) = permissions {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(permissions.mode() & 0o7777);
        }
        match options.open(&temporary) {
            Ok(file) => {
                if let Some(permissions) = permissions {
                    if let Err(error) = file.set_permissions(permissions.clone()) {
                        drop(file);
                        let _ = std::fs::remove_file(&temporary);
                        return Err(error);
                    }
                }
                return Ok((temporary, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// `std::fs::rename`, retried briefly on Windows.  There, replacing a file
/// fails while another process (antivirus, the search indexer, a sync
/// client) has it open for a moment.
#[cfg(windows)]
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    // Five attempts, waiting 5, 10, 20 and 40 ms in between.
    let mut delay = std::time::Duration::from_millis(5);
    for _ in 0..4 {
        match std::fs::rename(from, to) {
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied
                    || matches!(
                        error.raw_os_error(),
                        Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
                    ) =>
            {
                std::thread::sleep(delay);
                delay *= 2;
            }
            result => return result,
        }
    }
    std::fs::rename(from, to)
}

#[cfg(not(windows))]
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
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

    #[cfg(unix)]
    #[test]
    fn creates_the_target_of_dangling_symlinks() {
        let dir = TestDir::new("dangling");
        std::fs::create_dir(dir.0.join("splits")).unwrap();
        // A relative link resolves against the link's directory, not the
        // working directory, and chains are followed to the end.
        let link = dir.0.join("Link.lss");
        std::os::unix::fs::symlink("Middle.lss", &link).unwrap();
        std::os::unix::fs::symlink("splits/Real.lss", dir.0.join("Middle.lss")).unwrap();
        write_atomic_with_backup(&link, "one").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let target = dir.0.join("splits").join("Real.lss");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "one");

        // An absolute dangling link.
        let absolute = dir.0.join("Absolute.lss");
        let absolute_target = dir.0.join("splits").join("Other.lss");
        std::os::unix::fs::symlink(&absolute_target, &absolute).unwrap();
        write_atomic(&absolute, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&absolute_target).unwrap(), "two");

        // A loop is an error, not a hang.
        let looped = dir.0.join("Loop.lss");
        std::os::unix::fs::symlink("Loop.lss", &looped).unwrap();
        assert!(write_atomic(&looped, "three").is_err());
    }

    #[test]
    fn refuses_to_replace_read_only_files() {
        let dir = TestDir::new("readonly");
        let path = dir.0.join("Run.lss");
        write_atomic(&path, "pb").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();

        for result in [
            write_atomic(&path, "new"),
            write_atomic_with_backup(&path, "new"),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "pb");
        assert_eq!(dir.entries(), ["Run.lss"]);

        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_never_written_with_wider_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("private");
        let path = dir.0.join("Run.lss");
        write_atomic(&path, "one").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_with(&path, false, |file| {
            // The temporary file already has the final mode while the
            // contents are written.
            assert_eq!(file.metadata()?.permissions().mode() & 0o777, 0o600);
            file.write_all(b"two")
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    }
}
