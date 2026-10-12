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

const DURABILITY: &str = "
CREATE TABLE seat_restore (
    seat_id TEXT PRIMARY KEY NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
    posture TEXT NOT NULL,
    resume_kind TEXT,
    resume_value TEXT,
    transcript_cursor INTEGER,
    startup_status TEXT
);
CREATE TABLE seat_edges (
    rig_id TEXT NOT NULL REFERENCES rigs(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY (rig_id, source_id, target_id, kind)
);
CREATE TABLE checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
    summary TEXT NOT NULL,
    current_task TEXT,
    next_step TEXT,
    blocked_on TEXT,
    key_artifacts TEXT NOT NULL,
    confidence TEXT,
    created_ms INTEGER NOT NULL
);
CREATE INDEX idx_checkpoints_seat ON checkpoints (seat_id, created_ms);
CREATE TABLE direct_deliveries (
    id TEXT PRIMARY KEY NOT NULL,
    rig_id TEXT NOT NULL,
    seat_id TEXT NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    body TEXT NOT NULL,
    delivered_at_ms INTEGER,
    cursor INTEGER,
    acked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE rig_kernel (
    rig_id TEXT PRIMARY KEY NOT NULL REFERENCES rigs(id) ON DELETE CASCADE
);
";

/// Rig tables, queue tables, chat, then snapshot/restore tables.
pub static MIGRATIONS: LazyLock<MigrationSet> = LazyLock::new(|| MigrationSet {
    inner: Migrations::new(vec![
        M::up(SCHEMA),
        M::up(QUEUE_SCHEMA),
        M::up(CHAT),
        M::up(DURABILITY),
    ]),
    len: 4,
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
