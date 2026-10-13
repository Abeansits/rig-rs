// Derived from openrig packages/cli/src/commands/context.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig context`: the library verbs plus `deliver` for guarded paste and startup.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::error::Error;

/// Browse, preview, compose, and manage operator-authored context packs.
#[derive(Debug, Args)]
pub(crate) struct ContextArgs {
    #[command(subcommand)]
    command: ContextCmd,
}

#[derive(Debug, Subcommand)]
enum ContextCmd {
    /// Resolve System World plus project work context and the managed skill loadout.
    WorkInstall {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Walk the topology tree for one chain filename.
    Trace {
        /// Rig name.
        #[arg(long)]
        rig: String,
        /// Pod id. Omit when no pod is selected.
        #[arg(long)]
        pod: Option<String>,
        /// Seat folder. Omit for a rig-level trace.
        #[arg(long)]
        seat: Option<String>,
        /// Chain filename, identical at every altitude.
        #[arg(long)]
        name: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Compose ordered files into a durable context-pack ref.
    Compose {
        /// Path-like durable output ref.
        #[arg(long)]
        out: String,
        /// Ordered source files.
        #[arg(long = "from", required = true, num_args = 1..)]
        from: Vec<String>,
    },
    /// List all context packs in the library.
    List {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show pack manifest and per-file metadata.
    Show {
        /// Pack name or path-like ref.
        name_or_ref: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the assembled bundle without delivering it.
    Preview {
        /// Pack name or path-like ref.
        name_or_ref: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Serve the assembled bundle, or one addressed section.
    Get {
        /// Name, ref, or `<pack-ref>/<file>#H2-slug`.
        name_or_ref: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Compose a situation profile.
    Profile {
        /// Pack name or path-like ref.
        name_or_ref: String,
        /// `fresh`, `handover`, or `post-compaction`.
        #[arg(long)]
        situation: String,
        /// `claude`, `claude-code`, or `codex`.
        #[arg(long)]
        runtime: Option<String>,
        /// Named profile id.
        #[arg(long)]
        profile: Option<String>,
        /// Token budget. Overage is reported.
        #[arg(long)]
        budget: Option<String>,
        /// Topology rig segment.
        #[arg(long)]
        rig: Option<String>,
        /// Topology seat segment.
        #[arg(long)]
        seat: Option<String>,
        /// Mission segment.
        #[arg(long)]
        mission: Option<String>,
        /// Slice segment.
        #[arg(long)]
        slice: Option<String>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Write RECAP.md for one topology seat.
    RecapWrite {
        /// Rig segment.
        #[arg(long)]
        rig: String,
        /// Seat segment.
        #[arg(long)]
        seat: String,
        /// Markdown file to store.
        #[arg(long)]
        file: PathBuf,
    },
    /// Re-walk discovery roots.
    Sync {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Install a local context pack directory.
    Add {
        /// Directory that contains `manifest.yaml`.
        source: String,
        /// Install ref. Defaults to the manifest name.
        #[arg(long)]
        name: Option<String>,
        /// Git checkout. Not available in this build.
        #[arg(long)]
        git: bool,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect or update a Git-backed pack.
    Source {
        #[command(subcommand)]
        command: SourceCmd,
    },
    /// Remove a pack by its path-like ref.
    Rm {
        /// Path-like ref.
        refer: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Deliver a pack through the guarded paste path or `deliver_startup`.
    Deliver {
        /// Path-like ref.
        #[arg(long)]
        refer: String,
        /// Seat name or `seat@rig`.
        #[arg(long)]
        seat: String,
        /// `paste` or `startup`.
        #[arg(long)]
        mode: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SourceCmd {
    /// Read a Git selection. Not available in this build.
    Inspect {
        /// Pack ref.
        refer: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Fetch and merge a Git selection. Not available in this build.
    Update {
        /// Pack ref.
        refer: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn run(args: &ContextArgs) -> Result<i32, Error> {
    match &args.command {
        ContextCmd::WorkInstall { json } => Ok(work_install(*json)),
        ContextCmd::Trace {
            rig,
            pod,
            seat,
            name,
            json,
        } => crate::context_trace::trace(rig, pod.as_deref(), seat.as_deref(), name, *json),
        ContextCmd::Compose { out, from } => crate::context_view::compose(out, from),
        ContextCmd::List { json } => crate::context_view::list(*json),
        ContextCmd::Show { name_or_ref, json } => crate::context_view::show(name_or_ref, *json),
        ContextCmd::Preview { name_or_ref, json } => {
            crate::context_view::preview(name_or_ref, *json)
        }
        ContextCmd::Get { name_or_ref, json } => crate::context_view::get(name_or_ref, *json),
        ContextCmd::Profile {
            name_or_ref,
            situation,
            runtime,
            profile,
            budget,
            rig,
            seat,
            mission,
            slice,
            json,
        } => crate::context_view::profile(&crate::context_view::ProfileArgs {
            name_or_ref,
            situation,
            runtime: runtime.as_deref(),
            profile: profile.as_deref(),
            budget: budget.as_deref(),
            rig: rig.as_deref(),
            seat: seat.as_deref(),
            mission: mission.as_deref(),
            slice: slice.as_deref(),
            json: *json,
        }),
        ContextCmd::RecapWrite { rig, seat, file } => {
            crate::context_view::recap_write(rig, seat, file)
        }
        ContextCmd::Sync { json } => crate::context_view::sync(*json),
        ContextCmd::Add {
            source,
            name,
            git,
            json,
        } => crate::context_add::add(source, name.as_deref(), *git, *json),
        ContextCmd::Source { command } => source_refused(command),
        ContextCmd::Rm { refer, json } => crate::context_view::remove(refer, *json),
        ContextCmd::Deliver {
            refer,
            seat,
            mode,
            json,
        } => crate::context_view::deliver(refer, seat, mode, *json),
    }
}

fn work_install(json: bool) -> i32 {
    if json {
        crate::output::stdout_json(&serde_json::json!({
            "systemWorld": {"state": "absent", "source": "unconfigured"},
            "skills": [],
            "note": "System World skill loadout is not part of this build",
        }));
    } else {
        crate::output::stdout_line("system  absent [unconfigured]");
        crate::output::stdout_line("skills  system=(none)");
        crate::output::stdout_line("skills  topology=(none)");
        crate::output::stdout_line("skills  project=(none)");
    }
    0
}

fn source_refused(command: &SourceCmd) -> Result<i32, Error> {
    let json = match command {
        SourceCmd::Inspect { json, .. } | SourceCmd::Update { json, .. } => *json,
    };
    let message = "Git source selection is not available in this build; add a local directory with rig context add";
    if json {
        crate::output::stderr_line(&serde_json::json!({"error": message}).to_string());
    }
    Err(Error::User(message.to_owned()))
}
