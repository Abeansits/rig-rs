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
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut conn = match prepare(&path) {
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
