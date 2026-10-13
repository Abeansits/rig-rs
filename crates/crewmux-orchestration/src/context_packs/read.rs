// Derived from openrig packages/daemon/src/domain/context-packs/profile-source-resolver.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Fail-loud reads for library files and configured tree roots.

use std::path::{Path, PathBuf};

use crewmux_spec::context::{SourceKind, parse_source_ref};

use super::time::iso_from_system;

/// Configured tree roots. Missing means the compose was not granted that tree.
#[derive(Clone, Debug, Default)]
pub struct SourceRoots {
    /// Project tree.
    pub project: Option<PathBuf>,
    /// Seat tree.
    pub seat: Option<PathBuf>,
    /// Mission tree.
    pub mission: Option<PathBuf>,
}

/// Where the bytes actually came from.
#[derive(Clone, Debug)]
pub struct SourceRead {
    /// Pre-`#` ref.
    pub refer: String,
    /// Resolver kind.
    pub kind: SourceKind,
    /// Granted root.
    pub base: PathBuf,
    /// `base` joined with the relative path, before symlink resolution.
    pub nominal_path: PathBuf,
    /// `canonicalize` after a successful read.
    pub real_path: PathBuf,
    /// True when `real_path` is outside `base`.
    pub escapes_root: bool,
    /// Mtime of the real file, when it could be stated.
    pub written_at: Option<String>,
}

/// A read that did not produce bytes.
#[derive(Debug)]
pub enum ReadFail {
    /// The file is absent under a root that exists.
    Absent(String),
    /// Any other resolution failure, including a dangling symlink.
    Failed(String),
}

impl ReadFail {
    /// Sentence.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Absent(message) | Self::Failed(message) => message,
        }
    }
}

/// Read `refer` from the pack directory or a configured root.
///
/// # Errors
///
/// Returns [`ReadFail`] for a missing root, a traversal ref, a dangling
/// symlink, or an unreadable file.
pub fn read_source(
    pack_dir: &Path,
    roots: &SourceRoots,
    refer: &str,
) -> Result<(String, SourceRead), ReadFail> {
    let parsed = parse_source_ref(refer).map_err(|err| ReadFail::Failed(err.message))?;
    let base = base_for(pack_dir, roots, refer, parsed.kind)?;
    let abs = base.join(&parsed.rel);
    let text = match std::fs::read_to_string(&abs) {
        Ok(text) => text,
        Err(err) => return Err(read_error(refer, parsed.kind, &abs, &base, &err)),
    };
    let real_path = std::fs::canonicalize(&abs).unwrap_or_else(|_| abs.clone());
    let real_base = std::fs::canonicalize(&base).unwrap_or_else(|_| base.clone());
    let escapes_root = escapes(&real_base, &real_path);
    let written_at = std::fs::metadata(&real_path)
        .and_then(|meta| meta.modified())
        .ok()
        .map(iso_from_system);
    Ok((
        text,
        SourceRead {
            refer: refer.to_owned(),
            kind: parsed.kind,
            base,
            nominal_path: abs,
            real_path,
            escapes_root,
            written_at,
        },
    ))
}

fn base_for(
    pack_dir: &Path,
    roots: &SourceRoots,
    refer: &str,
    kind: SourceKind,
) -> Result<PathBuf, ReadFail> {
    let configured = match kind {
        SourceKind::Library => return Ok(pack_dir.to_path_buf()),
        SourceKind::Project => roots.project.as_ref(),
        SourceKind::Seat => roots.seat.as_ref(),
        SourceKind::Mission => roots.mission.as_ref(),
    };
    configured.cloned().ok_or_else(|| {
        ReadFail::Failed(format!(
            "source ref '{refer}' needs the {} tree root, which is not configured for this compose — \
             tree paths resolve from config, never literals; supply the {} root or drop the {}-homed atom.",
            kind.as_str(),
            kind.as_str(),
            kind.as_str()
        ))
    })
}

fn read_error(
    refer: &str,
    kind: SourceKind,
    abs: &Path,
    base: &Path,
    err: &std::io::Error,
) -> ReadFail {
    let meta = std::fs::symlink_metadata(abs).ok();
    let dangling = meta
        .as_ref()
        .is_some_and(|meta| meta.file_type().is_symlink())
        && err.kind() == std::io::ErrorKind::NotFound;
    if dangling {
        return ReadFail::Failed(format!(
            "source ref '{refer}' ({}) is a DANGLING symlink: {} exists but its target does not — {err}",
            kind.as_str(),
            abs.display()
        ));
    }
    let absent = meta.is_none() && err.kind() == std::io::ErrorKind::NotFound && base.is_dir();
    let message = format!(
        "source ref '{refer}' ({}) did not resolve: {} is unreadable — {err}",
        kind.as_str(),
        abs.display()
    );
    if absent {
        ReadFail::Absent(message)
    } else {
        ReadFail::Failed(message)
    }
}

fn escapes(base: &Path, real: &Path) -> bool {
    match real.strip_prefix(base) {
        Ok(inside) => inside.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        }),
        Err(_) => true,
    }
}
