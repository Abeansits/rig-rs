//! Schema. `user_version` equals [`MIGRATIONS`]`.len()` after a successful open.

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

    /// True when no migrations are embedded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

const CHAT: &str = "
CREATE TABLE chat_messages (
    id TEXT PRIMARY KEY NOT NULL,
    rig_id TEXT NOT NULL REFERENCES rigs(id) ON DELETE CASCADE,
    sender TEXT NOT NULL,
    body TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    topic TEXT
);
CREATE INDEX idx_chat_messages_rig_id ON chat_messages (rig_id, id);
CREATE TABLE seat_tokens (
    seat_id TEXT PRIMARY KEY NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
    token TEXT NOT NULL
);
";

/// Rig tables, then chat messages and seat tokens.
pub static MIGRATIONS: LazyLock<MigrationSet> = LazyLock::new(|| MigrationSet {
    inner: Migrations::new(vec![M::up(SCHEMA), M::up(CHAT)]),
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
