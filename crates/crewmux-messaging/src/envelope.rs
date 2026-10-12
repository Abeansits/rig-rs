// Derived from openrig packages/daemon/src/lib/pane-envelope.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Direct-message envelope. Each seat gets its own `To:` line.

/// Wrap `body` the way a recipient pane shows a direct send.
#[must_use]
pub fn wrap_pane(sender: &str, recipient: &str, body: &str) -> String {
    let sender = if sender.trim().is_empty() {
        "<unknown sender>"
    } else {
        sender
    };
    format!("From: {sender}\nTo: {recipient}\n---\n{body}\n---\n↩ Reply: rig send {sender} \"...\"")
}
