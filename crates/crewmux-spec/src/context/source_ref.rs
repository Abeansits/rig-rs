// Derived from openrig packages/daemon/src/domain/context-packs/profile-source-resolver.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Pre-`#` source refs: library paths and `project:` / `seat:` / `mission:` trees.

use super::markdown::{AddressError, parse_address};

/// Where an address's bytes live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// A file declared by the pack.
    Library,
    /// The configured project tree.
    Project,
    /// The configured seat tree.
    Seat,
    /// The configured mission tree.
    Mission,
}

impl SourceKind {
    /// TypeScript literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Project => "project",
            Self::Seat => "seat",
            Self::Mission => "mission",
        }
    }
}

/// A pre-`#` ref split into a resolver kind and a relative path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSourceRef {
    /// Resolver.
    pub kind: SourceKind,
    /// Path under the pack directory or the kind's configured root.
    pub rel: String,
}

/// Unknown prefix or a traversal-shaped path.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceRefError {
    /// TypeScript sentence.
    pub message: String,
}

impl std::fmt::Display for SourceRefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

const RECAP_FILENAME: &str = "RECAP.md";

/// Split `refer` into a kind and a relative path.
///
/// # Errors
///
/// Returns [`SourceRefError`] for an unknown prefix, an empty path, or a
/// traversal-shaped relative path.
pub fn parse_source_ref(refer: &str) -> Result<ParsedSourceRef, SourceRefError> {
    let (kind, rel) = split_kind(refer)?;
    if rel.is_empty() {
        return Err(fail(format!(
            "source ref '{refer}' has an empty path after its kind prefix."
        )));
    }
    if unsafe_rel(rel) {
        return Err(fail(format!(
            "source ref '{refer}' is traversal-shaped (absolute path, '..' or empty segment) — refs stay inside their source root."
        )));
    }
    Ok(ParsedSourceRef {
        kind,
        rel: rel.to_owned(),
    })
}

/// True for `seat:RECAP.md`, including a header path.
#[must_use]
pub fn is_seat_recap_address(address: &str) -> bool {
    let Ok(parsed) = parse_address(address) else {
        return false;
    };
    let Ok(source) = parse_source_ref(&parsed.refer) else {
        return false;
    };
    source.kind == SourceKind::Seat && source.rel == RECAP_FILENAME
}

/// Source label for one atom address. A malformed address is library.
///
/// # Errors
///
/// Returns the address or source-ref sentence when the address is malformed.
pub fn source_kind_for_address(address: &str) -> Result<SourceKind, String> {
    let parsed = parse_address(address).map_err(|err: AddressError| err.message)?;
    let source = parse_source_ref(&parsed.refer).map_err(|err| err.message)?;
    Ok(source.kind)
}

fn split_kind(refer: &str) -> Result<(SourceKind, &str), SourceRefError> {
    let Some((prefix, rest)) = refer.split_once(':') else {
        return Ok((SourceKind::Library, refer));
    };
    let kind = match prefix {
        "project" => SourceKind::Project,
        "seat" => SourceKind::Seat,
        "mission" => SourceKind::Mission,
        _ => {
            return Err(fail(format!(
                "source ref '{refer}' has unknown kind prefix '{prefix}' — the pre-'#' dialect is bare (library), 'project:' or 'seat:' or 'mission:'."
            )));
        }
    };
    Ok((kind, rest))
}

fn unsafe_rel(rel: &str) -> bool {
    if rel.starts_with('/') || rel.starts_with('\\') {
        return true;
    }
    rel.split(['\\', '/'])
        .any(|segment| segment.is_empty() || segment == "..")
}

fn fail(message: String) -> SourceRefError {
    SourceRefError { message }
}
