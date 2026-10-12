//! Per-rig chat: rows, history filters, and timestamp parsing.
//!
//! The store implements [`ChatRepo`]. Callers resolve the sender before `post`.

mod error;
mod model;
mod port;
mod since;

pub use error::ChatError;
pub use model::{ChatMessage, HistoryPage, HistoryQuery, NewChat, NewTopic};
pub use port::ChatRepo;
pub use since::{MAX_SINCE_LEN, escape_controls, format_sqlite_utc, parse_since, rejected_since};
