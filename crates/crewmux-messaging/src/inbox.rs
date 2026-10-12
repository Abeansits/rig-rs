// Derived from openrig packages/daemon/src/domain/inbox-handler.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Inbox drop / absorb / deny, and the outbox of recorded nudges.

use crewmux_core::Timestamp;
use crewmux_types::QItemId;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::session::SessionRef;

/// Inbox row state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxState {
    /// Waiting for the receiver.
    Pending,
    /// Promoted into the queue.
    Absorbed,
    /// Refused with a reason.
    Denied,
}

/// One dropped message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxEntry {
    /// Id.
    pub id: String,
    /// Receiver.
    pub destination: SessionRef,
    /// Sender, supplied by the transport.
    pub sender: SessionRef,
    /// Body.
    pub body: String,
    /// State.
    pub state: InboxState,
    /// When it was dropped.
    pub dropped_at: Timestamp,
    /// Queue item created by absorb.
    pub absorbed_qitem: Option<QItemId>,
    /// Deny reason.
    pub denied_reason: Option<String>,
}

/// A recorded outbound nudge or send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxRecord {
    /// Optional queue item.
    pub qitem: Option<QItemId>,
    /// Destination.
    pub destination: SessionRef,
    /// What was sent. For a wake this is the pointer, not the obligation.
    pub body: String,
    /// When.
    pub at: Timestamp,
}

/// In-memory inbox and outbox.
#[derive(Clone, Debug, Default)]
pub struct Mailboxes {
    inbox: Vec<InboxEntry>,
    outbox: Vec<OutboxRecord>,
}

impl Mailboxes {
    /// Empty mailboxes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop a message. A repeated id returns the existing row.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Rejected`] when `id` is empty.
    pub fn drop_inbox(&mut self, entry: InboxEntry) -> Result<InboxEntry, Error> {
        if entry.id.is_empty() {
            return Err(Error::Rejected {
                code: "inbox_id_required",
                message: "inbox id is required".to_owned(),
            });
        }
        if let Some(existing) = self.inbox.iter().find(|row| row.id == entry.id) {
            return Ok(existing.clone());
        }
        self.inbox.push(entry.clone());
        Ok(entry)
    }

    /// Mark a pending row absorbed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] or a state rejection.
    pub fn absorb(&mut self, id: &str, qitem: QItemId) -> Result<InboxEntry, Error> {
        let row = self.pending_mut(id)?;
        row.state = InboxState::Absorbed;
        row.absorbed_qitem = Some(qitem);
        Ok(row.clone())
    }

    /// Mark a pending row denied.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`], or a rejection when `reason` is empty.
    pub fn deny(&mut self, id: &str, reason: &str) -> Result<InboxEntry, Error> {
        if reason.trim().is_empty() {
            return Err(Error::Rejected {
                code: "deny_reason_required",
                message: "deny requires a reason".to_owned(),
            });
        }
        let row = self.pending_mut(id)?;
        row.state = InboxState::Denied;
        row.denied_reason = Some(reason.to_owned());
        Ok(row.clone())
    }

    /// Append an outbox row.
    pub fn record_outbox(&mut self, record: OutboxRecord) {
        self.outbox.push(record);
    }

    /// Every inbox row.
    #[must_use]
    pub fn inbox_rows(&self) -> Vec<InboxEntry> {
        self.inbox.clone()
    }

    /// Every outbox row.
    #[must_use]
    pub fn outbox_rows(&self) -> Vec<OutboxRecord> {
        self.outbox.clone()
    }

    /// Rebuild from persisted rows.
    #[must_use]
    pub fn restore(inbox: Vec<InboxEntry>, outbox: Vec<OutboxRecord>) -> Self {
        Self { inbox, outbox }
    }

    /// Outbox rows for one destination.
    #[must_use]
    pub fn outbox_for(&self, destination: &SessionRef) -> Vec<OutboxRecord> {
        self.outbox
            .iter()
            .filter(|row| &row.destination == destination)
            .cloned()
            .collect()
    }

    fn pending_mut(&mut self, id: &str) -> Result<&mut InboxEntry, Error> {
        let row = self
            .inbox
            .iter_mut()
            .find(|row| row.id == id)
            .ok_or_else(|| Error::NotFound(id.to_owned()))?;
        if row.state != InboxState::Pending {
            return Err(Error::Rejected {
                code: "inbox_not_pending",
                message: format!("inbox {id} is not pending"),
            });
        }
        Ok(row)
    }
}
