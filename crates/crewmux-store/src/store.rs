//! Single-writer SQLite actor. Reads share that connection so `:memory:` works.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::Connection;
use tokio::sync::Mutex;

use crate::error::Error;
use crate::migrate::migrate;

enum Msg {
    Job(Box<dyn FnOnce(&mut Connection) + Send>),
    Stop,
}

/// An open database. Share it with [`std::sync::Arc`]. The writer thread joins on drop.
///
/// The join handle sits behind a tokio mutex so the store is [`Sync`] without
/// using `std::sync::Mutex`. `try_lock` in [`Drop`] does not need a runtime.
pub struct Store {
    tx: Sender<Msg>,
    thread: Mutex<Option<JoinHandle<()>>>,
    path: std::path::PathBuf,
}

impl Store {
    /// Open `path`, set the pragmas, and apply migrations.
    ///
    /// `path` may be `:memory:`. A file from a newer schema is refused.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NewerSchema`] when `user_version` is past this build,
    /// or an I/O or SQLite error when the file cannot be opened.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let path = path.to_path_buf();
        let writer_path = path.clone();
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut conn = match prepare(&writer_path) {
                Ok(conn) => conn,
                Err(err) => {
                    drop(ready_tx.send(Err(err)));
                    return;
                }
            };
            drop(ready_tx.send(Ok(())));
            while let Ok(msg) = rx.recv() {
                match msg {
                    Msg::Job(job) => job(&mut conn),
                    Msg::Stop => break,
                }
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                thread: Mutex::new(Some(thread)),
                path,
            }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(Error::Closed),
        }
    }

    /// Run `f` on the writer connection.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Closed`] when the writer has stopped, or the error `f` returned.
    pub fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        self.call(f)
    }

    /// Run `f` on the same connection. Reads are serialised behind the writer.
    ///
    /// # Errors
    ///
    /// Same as [`write`](Self::write).
    pub fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        self.call(move |conn| f(conn))
    }

    /// Copy the database with the SQLite backup API on a new connection.
    ///
    /// This does not use the writer thread and does not run `wal_checkpoint`
    /// on it. `between` runs on the maintenance connection between pages.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backup`] for an in-memory database or a backup failure.
    pub fn backup_maintenance(&self, dest: &Path, mut between: impl FnMut()) -> Result<(), Error> {
        if self.path == Path::new(":memory:") {
            return Err(Error::Backup(
                "snapshots use the backup API on a file database, never the writer connection"
                    .to_owned(),
            ));
        }
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = temp_beside(dest)?;
        let copied = copy_backup(&self.path, &tmp, &mut between);
        if let Err(err) = copied {
            drop(std::fs::remove_file(&tmp));
            return Err(err);
        }
        if let Err(err) = Self::integrity_check(&tmp) {
            drop(std::fs::remove_file(&tmp));
            return Err(err);
        }
        sync_file(&tmp)?;
        std::fs::rename(&tmp, dest)?;
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
        {
            sync_dir(parent)?;
        }
        Ok(())
    }

    /// `PRAGMA integrity_check`. Anything other than `ok` is refused.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backup`] when the file is unreadable or the check fails.
    pub fn integrity_check(path: &Path) -> Result<(), Error> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|err| Error::Backup(format!("integrity_check: {err}")))?;
        let mut stmt = conn
            .prepare("PRAGMA integrity_check")
            .map_err(|err| Error::Backup(format!("integrity_check: {err}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| Error::Backup(format!("integrity_check: {err}")))?;
        let mut saw_ok = false;
        while let Some(row) = rows
            .next()
            .map_err(|err| Error::Backup(format!("integrity_check: {err}")))?
        {
            let text: String = row
                .get(0)
                .map_err(|err| Error::Backup(format!("integrity_check: {err}")))?;
            if text != "ok" {
                return Err(Error::Backup(format!("integrity_check: {text}")));
            }
            saw_ok = true;
        }
        if saw_ok {
            Ok(())
        } else {
            Err(Error::Backup("integrity_check returned no rows".to_owned()))
        }
    }

    /// `PRAGMA user_version` of a database file, without migrating it.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the file cannot be opened or the pragma cannot be read.
    pub fn file_user_version(path: &Path) -> Result<i64, Error> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok(version)
    }

    /// `PRAGMA user_version`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the pragma cannot be read.
    pub fn schema_version(&self) -> Result<i64, Error> {
        self.read(|conn| {
            let version = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
            Ok(version)
        })
    }

    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Msg::Job(Box::new(move |conn| {
                let result = f(conn);
                drop(reply_tx.send(result));
            })))
            .map_err(|_| Error::Closed)?;
        reply_rx.recv().map_err(|_| Error::Closed)?
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        drop(self.tx.send(Msg::Stop));
        let Ok(mut guard) = self.thread.try_lock() else {
            return;
        };
        if let Some(thread) = guard.take() {
            drop(thread.join());
        }
    }
}

fn prepare(path: &Path) -> Result<Connection, Error> {
    if path != Path::new(":memory:")
        && let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    migrate(&mut conn)?;
    private_db_files(path)?;
    Ok(conn)
}

/// `rig.db`, `rig.db-wal`, and `rig.db-shm` are owner-only.
fn private_db_files(path: &Path) -> Result<(), Error> {
    if path == Path::new(":memory:") {
        return Ok(());
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut owned = path.as_os_str().to_os_string();
        owned.push(suffix);
        let file = std::path::PathBuf::from(owned);
        match std::fs::metadata(&file) {
            Ok(_) => {
                let perms = std::fs::Permissions::from_mode(0o600);
                std::fs::set_permissions(&file, perms)?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

fn copy_backup(src_path: &Path, dest: &Path, between: &mut impl FnMut()) -> Result<(), Error> {
    let src = Connection::open(src_path)?;
    src.busy_timeout(Duration::from_millis(5000))?;
    let mut destination = Connection::open(dest)?;
    let backup = rusqlite::backup::Backup::new(&src, &mut destination)
        .map_err(|err| Error::Backup(format!("maintenance backup: {err}")))?;
    let mut retries = 0u32;
    loop {
        match backup.step(1) {
            Ok(rusqlite::backup::StepResult::Done) => return Ok(()),
            Ok(_) => {
                retries = 0;
                between();
            }
            Err(err) if sqlite_busy(&err) && retries < 20 => {
                retries = retries.saturating_add(1);
                std::thread::park_timeout(Duration::from_millis(25));
            }
            Err(err) => return Err(Error::Backup(format!("maintenance backup: {err}"))),
        }
    }
}

fn sqlite_busy(err: &rusqlite::Error) -> bool {
    let rusqlite::Error::SqliteFailure(code, _) = err else {
        return false;
    };
    matches!(
        code.code,
        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
    )
}

fn temp_beside(dest: &Path) -> Result<std::path::PathBuf, Error> {
    let Some(name) = dest.file_name().and_then(|name| name.to_str()) else {
        return Err(Error::Backup("snapshot path has no file name".to_owned()));
    };
    let mut tmp = dest.to_path_buf();
    tmp.set_file_name(format!(".{name}.tmp"));
    Ok(tmp)
}

fn sync_file(path: &Path) -> Result<(), Error> {
    let file = std::fs::File::open(path)?;
    file.sync_all()?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<(), Error> {
    let dir = std::fs::File::open(path)?;
    dir.sync_all()?;
    Ok(())
}
