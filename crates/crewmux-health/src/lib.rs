//! Health detectors, stream classifier, and Claude compaction control.
//!
//! The compaction enforcer decides when to ask for a restore map and when to
//! send `/compact`. Completion is a hook fact (PE-018), not a timer.

mod error;

pub mod compaction;

pub use error::Error;
