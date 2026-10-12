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

const HEALTH: &str = "
CREATE TABLE classifier_leases (
    id TEXT PRIMARY KEY NOT NULL,
    holder TEXT NOT NULL,
    acquired_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    last_heartbeat_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'expired', 'reclaimed'))
);
CREATE UNIQUE INDEX idx_classifier_leases_active_singleton
    ON classifier_leases(state) WHERE state = 'active';
CREATE TABLE stream_items (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    labels TEXT NOT NULL DEFAULT '{}',
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    source TEXT NOT NULL DEFAULT '',
    emitted_at_ms INTEGER NOT NULL
);
CREATE TABLE classification_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id TEXT NOT NULL,
    lease_id TEXT NOT NULL,
    holder TEXT NOT NULL,
    decision TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE health_policy (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    document TEXT NOT NULL
);
CREATE TABLE health_checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    body TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE health_diagnosis (
    id TEXT PRIMARY KEY NOT NULL,
    body TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE context_usage (
    seat TEXT PRIMARY KEY NOT NULL,
    used_tokens INTEGER,
    token_limit INTEGER CHECK (token_limit IS NULL OR token_limit > 0),
    sampled_at_ms INTEGER NOT NULL
);
";

/// Rig tables, the queue, chat, then health, stream, and classifier leases.
pub static MIGRATIONS: LazyLock<MigrationSet> = LazyLock::new(|| MigrationSet {
    inner: Migrations::new(vec![
        M::up(SCHEMA),
        M::up(QUEUE_SCHEMA),
        M::up(CHAT),
        M::up(HEALTH),
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
