//! Schema v2. `user_version` equals [`MIGRATIONS`]`.len()` after a successful open.

use std::sync::LazyLock;

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations, SchemaVersion};

use crate::error::Error;

const SCHEMA: &str = "
CREATE TABLE rigs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    created_ms INTEGER NOT NULL
);
CREATE TABLE seats (
    id TEXT PRIMARY KEY NOT NULL,
    rig_id TEXT NOT NULL REFERENCES rigs(id),
    name TEXT NOT NULL,
    runtime TEXT NOT NULL,
    cwd TEXT,
    model TEXT,
    state TEXT NOT NULL,
    attention_code TEXT,
    reason TEXT,
    tmux_session TEXT,
    pane_id TEXT,
    UNIQUE (rig_id, name)
);
CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id),
    tmux_session TEXT NOT NULL,
    pane_id TEXT,
    created_ms INTEGER NOT NULL
);
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    at_ms INTEGER NOT NULL
);
";

const QUEUE_SCHEMA: &str = "
CREATE TABLE queue_items (
    id TEXT PRIMARY KEY NOT NULL,
    state TEXT NOT NULL,
    updated_ms INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE queue_transitions (
    id INTEGER PRIMARY KEY NOT NULL,
    qitem TEXT NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE queue_items_archive (
    id TEXT PRIMARY KEY NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE queue_transitions_archive (
    id INTEGER PRIMARY KEY NOT NULL,
    qitem TEXT NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE inbox (
    id TEXT PRIMARY KEY NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    doc TEXT NOT NULL
);
CREATE TABLE delivery_receipts (
    cursor INTEGER PRIMARY KEY NOT NULL,
    doc TEXT NOT NULL
);
";

/// The migrations this build applies.
pub struct MigrationSet {
    inner: Migrations<'static>,
    len: usize,
}

impl MigrationSet {
    /// How many migrations are embedded.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when no migrations are embedded. This build always has some.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Schema v1 plus the queue, inbox, and receipt tables.
pub static MIGRATIONS: LazyLock<MigrationSet> = LazyLock::new(|| MigrationSet {
    inner: Migrations::new(vec![M::up(SCHEMA), M::up(QUEUE_SCHEMA)]),
    len: 2,
});

pub(crate) fn migrate(conn: &mut Connection) -> Result<(), Error> {
    match MIGRATIONS.inner.current_version(conn)? {
        SchemaVersion::Outside(found) => {
            return Err(Error::NewerSchema {
                found: found.get(),
                supported: MIGRATIONS.len(),
            });
        }
        SchemaVersion::NoneSet | SchemaVersion::Inside(_) => {}
    }
    MIGRATIONS.inner.to_latest(conn)?;
    Ok(())
}
