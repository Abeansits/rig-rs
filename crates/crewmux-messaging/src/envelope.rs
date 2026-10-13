// Derived from openrig packages/daemon/src/lib/pane-envelope.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Direct-message envelope. Each seat gets its own `To:` line.

/// Wrap `body` the way a recipient pane shows a direct send.
///
/// The reply hint addresses `sender`. Use [`wrap_reply`] when that hint should
/// be a `seat@rig` address and `sender` is a seat id.
#[must_use]
pub fn wrap_pane(sender: &str, recipient: &str, body: &str) -> String {
    wrap_reply(sender, recipient, body, sender)
}

/// Like [`wrap_pane`], with the reply hint addressed to `reply`.
#[must_use]
pub fn wrap_reply(sender: &str, recipient: &str, body: &str, reply: &str) -> String {
    let sender = shown(sender);
    let reply = if reply.trim().is_empty() {
        sender
    } else {
        reply
    };
    format!("From: {sender}\nTo: {recipient}\n---\n{body}\n---\n↩ Reply: rig send {reply} \"...\"")
}

fn shown(sender: &str) -> &str {
    if sender.trim().is_empty() {
        "<unknown sender>"
    } else {
        sender
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_reply;

    #[test]
    fn reply_hint_uses_the_address() {
        let text = wrap_reply("01ARZ3NDEKTSV4RRFFQ69G5FAV", "qa@demo", "hi", "impl@demo");
        assert!(text.contains("From: 01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(text.contains("↩ Reply: rig send impl@demo \"...\""));
        assert!(!text.contains("↩ Reply: rig send 01ARZ"));
    }
}
