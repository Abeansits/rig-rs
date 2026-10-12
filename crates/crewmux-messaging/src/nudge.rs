// Derived from openrig packages/daemon/src/domain/queue-nudge-text.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Wake text is a pointer at the row. The obligation body never rides along.

use crewmux_types::QItemId;

/// Pointer the ladder delivers. Style: `rig queue show <id>`.
#[must_use]
pub fn nudge_pointer(id: &QItemId) -> String {
    format!("you have queue item {id} — run `rig queue show {id}`")
}

/// Stable fingerprint of a direct send or a queue obligation.
#[must_use]
pub fn obligation_fingerprint(source: &str, destination: &str, body: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in source
        .bytes()
        .chain(std::iter::once(0))
        .chain(destination.bytes())
        .chain(std::iter::once(0))
        .chain(body.bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// True when `text` contains `body` as a substring. Empty bodies do not count.
#[must_use]
pub fn leaks_body(text: &str, body: &str) -> bool {
    !body.is_empty() && text.contains(body)
}

/// Queue rows whose last nudge was delivered and not confirmed.
///
/// Restore re-nudges these with [`nudge_pointer`] and does not change the row.
#[must_use]
pub fn unconfirmed_ids(items: &[crate::model::QueueItem]) -> Vec<QItemId> {
    items
        .iter()
        .filter(|item| {
            item.last_nudge
                .as_ref()
                .is_some_and(|nudge| nudge.result == "delivered-ack-pending")
        })
        .map(|item| item.id.clone())
        .collect()
}
