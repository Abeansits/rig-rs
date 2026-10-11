//! SQLite persistence: schema v1, a single writer, and the core repository ports.

mod error;
mod migrate;
mod queue;
mod repo;
mod store;

pub use error::Error;
pub use migrate::MIGRATIONS;
pub use queue::{archive_terminal, insert_receipt, load_book, max_receipt_cursor, replace_book};
pub use repo::{Db, clear_rig};
pub use store::Store;
