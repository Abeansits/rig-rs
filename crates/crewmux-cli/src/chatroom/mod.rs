// Derived from openrig packages/cli/src/commands/chatroom.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig chatroom send|history|watch|topic|clear`.

use clap::{Parser, Subcommand};
use crewmux_api::RigSummaryRow;

use crate::error::Error;

mod http;
mod watch;

/// `rig chatroom`.
#[derive(Debug, Parser)]
pub(crate) struct ChatroomArgs {
    #[command(subcommand)]
    command: ChatCmd,
}

#[derive(Debug, Subcommand)]
enum ChatCmd {
    /// Post a message. The sender is the seat token, or `operator`.
    Send {
        /// Rig name.
        rig: String,
        /// Message text.
        message: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print history. Without `--after`, the newest `limit` rows, oldest first.
    History {
        /// Rig name.
        rig: String,
        /// Topic window.
        #[arg(long)]
        topic: Option<String>,
        /// Return rows after this message id.
        #[arg(long)]
        after: Option<String>,
        /// Inclusive timestamp lower bound.
        #[arg(long)]
        since: Option<String>,
        /// Exact sender.
        #[arg(long)]
        sender: Option<String>,
        /// Maximum rows.
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print new messages until Ctrl-C.
    Watch {
        /// Rig name.
        rig: String,
        /// Print each SSE envelope as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Insert a topic marker.
    Topic {
        /// Rig name.
        rig: String,
        /// Topic name.
        name: String,
        /// Optional marker text.
        #[arg(long)]
        body: Option<String>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Delete every message in the rig.
    Clear {
        /// Rig name.
        rig: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn run(args: &ChatroomArgs) -> Result<i32, Error> {
    match &args.command {
        ChatCmd::Send { rig, message, json } => http::send(rig, message, *json),
        ChatCmd::History {
            rig,
            topic,
            after,
            since,
            sender,
            limit,
            json,
        } => http::history(&http::HistoryRequest {
            rig,
            topic: topic.as_deref(),
            after: after.as_deref(),
            since: since.as_deref(),
            sender: sender.as_deref(),
            limit: *limit,
            json: *json,
        }),
        ChatCmd::Watch { rig, json } => watch::run(rig, *json),
        ChatCmd::Topic {
            rig,
            name,
            body,
            json,
        } => http::topic(rig, name, body.as_deref(), *json),
        ChatCmd::Clear { rig, json } => http::clear(rig, *json),
    }
}

pub(crate) fn resolve_name<'a>(rows: &'a [RigSummaryRow], name: &str) -> Result<&'a str, String> {
    let matched: Vec<_> = rows.iter().filter(|row| row.name == name).collect();
    match matched.as_slice() {
        [] => Err(format!("Rig '{name}' not found. List rigs with: rig ps")),
        [one] => Ok(one.id.as_str()),
        many => Err(format!(
            "Rig '{name}' is ambiguous — {} rigs share that name. Use a unique name or remove duplicates.",
            many.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_name;
    use crewmux_api::RigSummaryRow;

    fn row(id: &str, name: &str) -> RigSummaryRow {
        RigSummaryRow {
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn missing_and_ambiguous_rig_names() {
        let rows = vec![row("a", "pair"), row("b", "pair"), row("c", "solo")];
        assert_eq!(resolve_name(&rows, "solo").expect("solo"), "c");
        let missing = resolve_name(&rows, "nope").expect_err("missing");
        assert_eq!(missing, "Rig 'nope' not found. List rigs with: rig ps");
        let ambiguous = resolve_name(&rows, "pair").expect_err("ambiguous");
        assert_eq!(
            ambiguous,
            "Rig 'pair' is ambiguous — 2 rigs share that name. Use a unique name or remove duplicates."
        );
    }
}
