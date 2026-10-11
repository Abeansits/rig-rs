//! Human tmux clients.
//!
//! Raw `session_attached` (and `#{session_attached}`) counts our control-mode
//! client as a person watching the session. ADR-0005 bans that format outside
//! this doc comment. Callers use [`human_clients`](crate::Multiplexer::human_clients),
//! which keeps only rows whose `client_control_mode` is 0.

use std::time::SystemTime;

use crate::parse::{human_clients_in, latest_human_activity};
use crate::types::ClientInfo;

/// Clients attached to `session` that are not the control-mode client.
#[must_use]
pub(crate) fn filter_human_clients(clients: &[ClientInfo], session: &str) -> Vec<ClientInfo> {
    human_clients_in(clients, session)
        .into_iter()
        .cloned()
        .collect()
}

/// Newest human keystroke time on `session`, or `None` when nobody human is attached.
#[must_use]
pub(crate) fn last_human_keystroke(clients: &[ClientInfo], session: &str) -> Option<SystemTime> {
    latest_human_activity(clients, session)
}
