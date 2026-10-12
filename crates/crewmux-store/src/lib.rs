//! SQLite persistence: schema, a single writer, and the core repository ports.

mod attempt;
mod chat;
mod durability;
mod error;
mod health_lease;
mod health_rows;
mod health_stream;
mod import;
mod lineage;
mod migrate;
mod queue;
mod repo;
mod store;
mod token;

pub use attempt::{AttemptRow, delete_attempt, get_attempt, list_attempts, upsert_attempt};
pub use durability::{
    CheckpointRow, DirectRow, EdgeRow, SeatRestoreRow, insert_checkpoint, insert_direct,
    list_checkpoints, list_direct, list_edges, load_seat_restore, mark_kernel, put_edge,
    put_seat_restore, rigs_kernel, set_direct_acked, set_direct_delivered,
};
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
pub use import::import_rig;
pub use lineage::{PaneLineage, lineage_get, lineage_put};
pub use migrate::MIGRATIONS;
pub use queue::{archive_terminal, insert_receipt, load_book, max_receipt_cursor, replace_book};
pub use repo::{Db, clear_rig};
pub use rusqlite::Connection;
pub use store::Store;
pub use token::{put_seat_token, seat_token};
