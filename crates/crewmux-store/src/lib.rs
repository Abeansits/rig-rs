//! SQLite persistence: schema, a single writer, and the core repository ports.

mod chat;
mod error;
mod health_lease;
mod health_rows;
mod health_stream;
mod lineage;
mod migrate;
mod queue;
mod repo;
mod store;
mod token;

pub use error::Error;
pub use health_lease::{
    LeaseClock, lease_acquire, lease_active, lease_evaluate, lease_heartbeat, lease_reclaim,
};
pub use health_rows::{
    UsageRow, checkpoint_list, checkpoint_put, diagnosis_get, diagnosis_list, diagnosis_put,
    policy_document, policy_load, policy_store, seat_states, usage_list, usage_upsert,
};
pub use health_stream::{
    Emitted, NewStream, attempt_count, attempt_insert, stream_archive, stream_emit, stream_get,
    stream_list,
};
pub use lineage::{PaneLineage, lineage_get, lineage_put};
pub use migrate::MIGRATIONS;
pub use queue::{archive_terminal, insert_receipt, load_book, max_receipt_cursor, replace_book};
pub use repo::{Db, clear_rig};
pub use rusqlite::Connection;
pub use store::Store;
pub use token::{put_seat_token, seat_token};
