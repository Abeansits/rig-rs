//! SQLite persistence: schema, a single writer, and the core repository ports.

mod chat;
mod durability;
mod error;
mod import;
mod migrate;
mod queue;
mod repo;
mod store;
mod token;

pub use durability::{
    CheckpointRow, DirectRow, EdgeRow, SeatRestoreRow, insert_checkpoint, insert_direct,
    list_checkpoints, list_direct, list_edges, load_seat_restore, mark_kernel, put_edge,
    put_seat_restore, rigs_kernel, set_direct_acked, set_direct_delivered,
};
pub use error::Error;
pub use import::import_rig;
pub use migrate::MIGRATIONS;
pub use queue::{archive_terminal, insert_receipt, load_book, max_receipt_cursor, replace_book};
pub use repo::{Db, clear_rig};
pub use rusqlite::Connection;
pub use store::Store;
pub use token::{put_seat_token, seat_token};
