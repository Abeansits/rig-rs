//! Directories, the lifetime lock, and the Unix socket.
//!
//! The lock is an `flock` on `run/daemon.lock`, held until the daemon exits.
//! A second start fails immediately, including the window before the socket
//! listens. Liveness of an existing socket is a connect, never a pid file:
//! a recycled pid must not block the next start.

use std::fs::OpenOptions;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use crate::error::Error;

/// Exclusive lock held for the daemon process lifetime.
#[must_use = "dropping the lock releases the daemon"]
#[derive(Debug)]
pub(crate) struct DaemonLock {
    file: std::fs::File,
}

impl DaemonLock {
    /// Lock `run/daemon.lock`. Fails when another daemon holds it.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the file cannot be opened or the lock is taken.
    pub(crate) fn acquire(home: &Path) -> Result<Self, Error> {
        let path = home.join("run").join("daemon.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Self { file }),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Err(Error::Message(
                format!("daemon is already running at {}", path.display()),
            )),
            Err(err) => Err(Error::Io(err.into())),
        }
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.file, rustix::fs::FlockOperation::Unlock);
    }
}

/// Create `RIG_HOME`, `state`, and `run` when missing, mode 0700.
///
/// An existing directory is left alone. Parents outside `home` are created
/// when needed and never chmod'd.
///
/// # Errors
///
/// Returns [`Error`] when a directory cannot be created.
pub(crate) fn prepare_dirs(home: &Path) -> Result<(), Error> {
    create_private(home)?;
    create_private(&home.join("state"))?;
    create_private(&home.join("run"))?;
    Ok(())
}

/// Bind `path` after refusing a connectable socket.
///
/// # Errors
///
/// Returns [`Error`] when the directory is unsafe or the socket is live.
pub(crate) fn bind_socket(path: &Path) -> Result<tokio::net::UnixListener, Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let created = create_private(parent)?;
        if !created && is_fallback_dir(parent) {
            require_private_dir(parent)?;
        }
    }
    if socket_connects(path) {
        return Err(Error::Message(format!(
            "daemon is already running at {}",
            path.display()
        )));
    }
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(listener)
}

/// `true` when this call created `dir` and chmod'd it to 0700.
fn create_private(dir: &Path) -> Result<bool, Error> {
    if let Some(parent) = dir.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::create_dir(dir) {
        Ok(()) => {
            set_mode(dir, 0o700)?;
            Ok(true)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(err.into()),
    }
}

fn set_mode(dir: &Path, mode: u32) -> Result<(), Error> {
    let mut perms = std::fs::metadata(dir)?.permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(dir, perms)?;
    Ok(())
}

/// `$TMPDIR/rig-<uid>` or `/tmp/rig-<uid>`, the short-socket fallback.
fn is_fallback_dir(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name != format!("rig-{}", rustix::process::getuid().as_raw()) {
        return false;
    }
    let Some(parent) = dir.parent() else {
        return false;
    };
    parent == Path::new("/tmp") || parent == std::env::temp_dir()
}

/// Refuse a fallback directory we did not just create unless we own it at 0700.
fn require_private_dir(dir: &Path) -> Result<(), Error> {
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.file_type().is_dir() {
        return Err(Error::Message(format!(
            "refusing socket directory {}: not a private directory",
            dir.display()
        )));
    }
    let mode = meta.mode() & 0o777;
    let owner = meta.uid();
    let ours = rustix::process::getuid().as_raw();
    if owner != ours || mode != 0o700 {
        return Err(Error::Message(format!(
            "refusing socket directory {}: owner {owner} mode {mode:04o}, want uid {ours} mode 0700",
            dir.display()
        )));
    }
    Ok(())
}

fn socket_connects(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::{bind_socket, create_private, is_fallback_dir, prepare_dirs, require_private_dir};

    fn scratch(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("rig-dirs-{label}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).expect("scratch");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("mode");
        path
    }

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).expect("meta").permissions().mode() & 0o777
    }

    #[test]
    fn prepare_dirs_chmods_only_directories_it_creates() {
        let root = scratch("prep");
        let home = root.join("home");
        prepare_dirs(&home).expect("dirs");
        assert_eq!(mode_of(&root), 0o755);
        assert_eq!(mode_of(&home), 0o700);
        assert_eq!(mode_of(&home.join("run")), 0o700);
        assert_eq!(mode_of(&home.join("state")), 0o700);
        let mut perms = std::fs::metadata(home.join("run"))
            .expect("run")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(home.join("run"), perms).expect("widen");
        prepare_dirs(&home).expect("again");
        assert_eq!(mode_of(&home.join("run")), 0o755);
        drop(std::fs::remove_dir_all(&root));
    }

    #[tokio::test]
    async fn shared_parent_keeps_its_mode() {
        let root = scratch("mode");
        let private = root.join("rig-test");
        let sock = private.join("daemon.sock");
        let listener = bind_socket(&sock).expect("bind");
        drop(listener);
        assert_eq!(
            mode_of(&root),
            0o755,
            "refusing to chmod a shared directory"
        );
        assert_eq!(mode_of(&private), 0o700);
        drop(std::fs::remove_dir_all(&root));
    }

    #[tokio::test]
    async fn existing_run_directory_keeps_its_mode() {
        let root = scratch("run");
        let run = root.join("run");
        std::fs::create_dir(&run).expect("run");
        let mut perms = std::fs::metadata(&run).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&run, perms).expect("mode");
        let listener = bind_socket(&run.join("daemon.sock")).expect("bind");
        drop(listener);
        assert_eq!(mode_of(&run), 0o755);
        assert_eq!(mode_of(&root), 0o755);
        drop(std::fs::remove_dir_all(&root));
    }

    #[test]
    fn live_socket_is_not_unlinked() {
        let root = scratch("live");
        let private = root.join("run");
        std::fs::create_dir_all(&private).expect("run");
        let sock = private.join("daemon.sock");
        let held = std::os::unix::net::UnixListener::bind(&sock).expect("listen");
        let err = bind_socket(&sock).expect_err("live");
        assert!(err.to_string().contains("already running"), "{err}");
        assert!(sock.exists());
        drop(held);
        drop(std::fs::remove_dir_all(&root));
    }

    #[test]
    fn fallback_name_is_only_our_uid_under_temp() {
        let uid = rustix::process::getuid().as_raw();
        let dir = std::env::temp_dir().join(format!("rig-{uid}"));
        assert!(is_fallback_dir(&dir));
        assert!(is_fallback_dir(&std::path::PathBuf::from(format!(
            "/tmp/rig-{uid}"
        ))));
        assert!(!is_fallback_dir(&std::env::temp_dir().join("run")));
        assert!(!is_fallback_dir(&std::env::temp_dir().join("rig-test")));
    }

    #[test]
    fn open_or_foreign_fallback_is_refused() {
        let root = scratch("own");
        let dir = root.join("socket-dir");
        assert!(create_private(&dir).expect("create"));
        let mut perms = std::fs::metadata(&dir).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dir, perms).expect("mode");
        let err = require_private_dir(&dir).expect_err("open");
        assert!(err.to_string().contains("0700"), "{err}");
        let mut perms = std::fs::metadata(&dir).expect("meta").permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&dir, perms).expect("mode");
        require_private_dir(&dir).expect("private");
        let link = root.join("link");
        std::os::unix::fs::symlink(&dir, &link).expect("symlink");
        let err = require_private_dir(&link).expect_err("symlink");
        assert!(err.to_string().contains("not a private directory"), "{err}");
        drop(std::fs::remove_dir_all(&root));
    }

    #[test]
    fn second_lock_is_refused_until_the_first_drops() {
        let root = scratch("lock");
        let home = root.join("home");
        prepare_dirs(&home).expect("dirs");
        let held = super::DaemonLock::acquire(&home).expect("lock");
        let err = super::DaemonLock::acquire(&home).expect_err("busy");
        assert!(err.to_string().contains("already running"), "{err}");
        drop(held);
        let again = super::DaemonLock::acquire(&home).expect("free");
        drop(again);
        drop(std::fs::remove_dir_all(&root));
    }
}
