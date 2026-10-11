//! Ports the store implements. Methods are synchronous and run inside one transaction.

use crewmux_types::{RigId, RigName, SeatId};

use crate::error::Error;
use crate::event::{DomainEvent, Seq};
use crate::model::{Rig, Seat};

/// Rig persistence.
pub trait RigRepo {
    /// Fetch one rig.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RigNotFound`] or [`Error::Store`].
    fn get(&self, id: &RigId) -> Result<Rig, Error>;

    /// Fetch by spec name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn get_by_name(&self, name: &RigName) -> Result<Option<Rig>, Error>;

    /// Insert or replace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn upsert(&self, rig: &Rig) -> Result<(), Error>;

    /// Every rig, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn list(&self) -> Result<Vec<Rig>, Error>;
}

/// Seat persistence.
pub trait SeatRepo {
    /// Fetch one seat.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SeatNotFound`] or [`Error::Store`].
    fn get(&self, id: &SeatId) -> Result<Seat, Error>;

    /// Seats of one rig, in insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn list_by_rig(&self, rig: &RigId) -> Result<Vec<Seat>, Error>;

    /// Every seat.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn list_all(&self) -> Result<Vec<Seat>, Error>;

    /// Insert or replace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn upsert(&self, seat: &Seat) -> Result<(), Error>;

    /// The seat bound to a tmux pane id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn get_by_pane(&self, pane_id: &str) -> Result<Option<Seat>, Error>;
}

/// Session rows. The lifecycle stores the tmux name on the seat; this records the id.
pub trait SessionRepo {
    /// Record a session that was actually created.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn insert(
        &self,
        id: &crewmux_types::SessionId,
        seat: &SeatId,
        tmux_session: &str,
        pane_id: Option<&str>,
        at: crate::clock::Timestamp,
    ) -> Result<(), Error>;
}

/// Append-only event log.
pub trait EventLog {
    /// Append one event and return its sequence.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn append(&self, event: &DomainEvent) -> Result<Seq, Error>;

    /// Events in causal (append) order.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] on persistence failure.
    fn list(&self) -> Result<Vec<DomainEvent>, Error>;
}
