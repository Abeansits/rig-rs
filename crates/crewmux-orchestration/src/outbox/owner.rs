// Derived from openrig packages/daemon/src/domain/queue-repository.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Owner-wake rows. A recorded nudge is an obligation; the paste is the pointer.

use crewmux_messaging::{QueueBook, nudge_pointer};
use crewmux_types::QItemId;

use crate::model::Engine;

/// One successor whose nudge was recorded and not yet pasted.
pub struct OwnerWake {
    /// Stable job id: `owner-wake:` plus the packet id.
    pub job_id: String,
    /// Frontier packet.
    pub packet_id: String,
    /// Destination session.
    pub target: String,
    /// Pointer text. The obligation body is not in it.
    pub message: String,
}

/// Recorded nudges on live frontiers.
#[must_use]
pub fn owner_wakes(engine: &Engine, book: &QueueBook) -> Vec<OwnerWake> {
    let mut out = Vec::new();
    for instance in &engine.instances {
        if !instance.status.is_live() {
            continue;
        }
        for packet in &instance.frontier {
            if let Some(wake) = one(book, packet) {
                out.push(wake);
            }
        }
    }
    out
}

fn one(book: &QueueBook, packet: &QItemId) -> Option<OwnerWake> {
    let item = book.get(packet)?;
    let recorded = item
        .last_nudge
        .as_ref()
        .is_some_and(|nudge| nudge.result == "recorded");
    if !recorded {
        return None;
    }
    let message = nudge_pointer(packet);
    Some(OwnerWake {
        job_id: format!("owner-wake:{packet}"),
        packet_id: packet.as_str().to_owned(),
        target: item.destination.as_str().to_owned(),
        message,
    })
}
