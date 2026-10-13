// Derived from openrig packages/daemon/src/domain/context-packs/context-pack-library-service.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Filesystem context-pack library. The index is memory only.

mod assemble;
mod builtin;
mod compose;
mod entry;
mod error;
mod library;
mod probe;
mod read;
mod recap;
mod time;
mod tokens;
mod write;

pub use assemble::{assemble_bundle, assemble_plain};
pub use builtin::materialize_builtin;
pub use compose::{
    ComposeError, ComposedPiece, ComposedProfile, ContextAtoms, NamedCompose,
    compose_named_profile, compose_profile,
};
pub use entry::{EntryFile, PackEntry, SourceType, context_pack_id};
pub use error::{PackCode, PackError};
pub use library::{Library, Root, SyncReport};
pub use probe::{CompiledProbes, compile_atom_probes};
pub use read::{ReadFail, SourceRead, SourceRoots, read_source};
pub use recap::{
    ContractFinding, RECAP_FILENAME, RecapError, list_recap_chain, validate_recap_contract,
    write_seat_recap,
};
pub use tokens::{estimate_tokens, estimate_tokens_from_bytes};
pub use write::{ComposeFile, ComposeOutcome, RemoveOutcome};

/// Home roots in TypeScript startup order: builtin, then the user store, then
/// the overlapping `system` root. Workspace roots are added by the daemon.
#[must_use]
pub fn home_roots(home: &std::path::Path) -> Vec<Root> {
    let mut roots = Vec::new();
    let builtin = home.join("builtin-context-packs");
    if builtin.is_dir() {
        roots.push(Root::new(builtin, SourceType::Builtin));
    }
    let context = home.join("context");
    roots.push(Root::new(context.clone(), SourceType::UserFile));
    roots.push(Root::new(context.join("system"), SourceType::UserFile));
    roots
}
