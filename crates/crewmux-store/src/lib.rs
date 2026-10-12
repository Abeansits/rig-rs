//! SQLite persistence: schema, a single writer, and the core repository ports.

mod attempt;
mod chat;
mod error;
mod migrate;
mod queue;
mod repo;
mod store;
mod token;

pub use attempt::{AttemptRow, delete_attempt, get_attempt, list_attempts, upsert_attempt};
pub use error::Error;
pub use migrate::MIGRATIONS;
pub use queue::{archive_terminal, insert_receipt, load_book, max_receipt_cursor, replace_book};
pub use repo::{Db, clear_rig};
pub use store::Store;
pub use token::{put_seat_token, seat_token};
