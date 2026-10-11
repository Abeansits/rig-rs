//! SQLite persistence: schema v1, a single writer, and the core repository ports.

mod error;
mod migrate;
mod repo;
mod store;

pub use error::Error;
pub use migrate::MIGRATIONS;
pub use repo::{Db, clear_rig};
pub use store::Store;
