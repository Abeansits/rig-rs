//! Queue unit of work. Slice 18 calls [`QueueRepo::update_within_transaction`].
//!
//! The transaction is the implementor. `crewmux-store` runs these methods inside
//! one SQLite transaction; this crate does not name a database.

use crewmux_core::Timestamp;
use crewmux_types::QItemId;

use crate::book::QueueBook;
use crate::error::Error;
use crate::model::{NewItem, QueueItem, Transition, Update};

/// Create and update a queue inside one unit of work.
pub trait QueueRepo {
    /// Insert a pending item.
    ///
    /// # Errors
    ///
    /// Returns a crash or a rejected id.
    fn create(&mut self, new: NewItem, now: Timestamp) -> Result<QueueItem, Error>;

    /// Apply one state write. The caller commits or rolls back around this call.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection when the transition is illegal.
    fn update_within_transaction(
        &mut self,
        id: &QItemId,
        update: Update,
        now: Timestamp,
    ) -> Result<Transition, Error>;
}

impl QueueRepo for QueueBook {
    fn create(&mut self, new: NewItem, now: Timestamp) -> Result<QueueItem, Error> {
        Self::create(self, new, now)
    }

    fn update_within_transaction(
        &mut self,
        id: &QItemId,
        update: Update,
        now: Timestamp,
    ) -> Result<Transition, Error> {
        self.update(id, update, now)
    }
}
