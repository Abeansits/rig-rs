// Derived from openrig packages/daemon/src/domain/context-packs/context-pack-library-service.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Durable compose and removal. Safety checks run before any mutation.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::assemble::{PlainFileStat, PlainInput, assemble_plain};
use super::entry::{PackEntry, SourceType};
use super::error::{PackCode, PackError};
use super::library::Library;

/// One ordered compose source.
pub struct ComposeFile {
    /// Absolute source path.
    pub path: PathBuf,
    /// Caller spelling, stored as the member summary.
    pub label: String,
}

/// What `compose_from_files` wrote and the plain projection of those bytes.
pub struct ComposeOutcome {
    /// Durable ref.
    pub refer: String,
    /// Indexed entry after the rescan.
    pub entry: PackEntry,
    /// Present members joined by a blank line.
    pub text: String,
    /// UTF-8 length of `text`.
    pub bytes: u64,
    /// Token estimate.
    pub estimated_tokens: u32,
    /// Present members.
    pub files: Vec<PlainFileStat>,
}

/// What `remove_by_ref` deleted.
pub struct RemoveOutcome {
    /// Always true on the Ok path.
    pub removed: bool,
    /// The ref.
    pub refer: String,
    /// Directory that was removed.
    pub removed_path: PathBuf,
}

impl Library {
    /// Compose ordered files into one user-store pack.
    ///
    /// # Errors
    ///
    /// Returns [`PackError`] for an unsafe ref, a missing source, a conflict,
    /// or a write failure. A failed write removes the new directory.
    pub fn compose_from_files(
        &mut self,
        out_ref: &str,
        sources: &[ComposeFile],
    ) -> Result<ComposeOutcome, PackError> {
        assert_ref(out_ref)?;
        if sources.is_empty() {
            return Err(PackError::with(
                PackCode::MissingFiles,
                "context composition requires at least one --from file",
                json!({"missingFiles": []}),
            ));
        }
        let missing: Vec<String> = sources
            .iter()
            .filter(|source| !source.path.is_file())
            .map(|source| source.path.display().to_string())
            .collect();
        if !missing.is_empty() {
            return Err(PackError::with(
                PackCode::MissingFiles,
                format!(
                    "context composition source file(s) not found: {}",
                    missing.join(", ")
                ),
                json!({"missingFiles": missing}),
            ));
        }
        let members = read_members(sources)?;
        let write_root = self
            .roots()
            .iter()
            .find(|root| root.source_type == SourceType::UserFile)
            .map(|root| root.path.clone())
            .ok_or_else(|| {
                PackError::new(
                    PackCode::StoreUnavailable,
                    "context pack composition requires a writable user_file store root",
                )
            })?;
        drop(self.scan());
        if self.contains(out_ref) {
            return Err(PackError::with(
                PackCode::PackExists,
                format!("context pack ref '{out_ref}' already exists in the library"),
                json!({"ref": out_ref}),
            ));
        }
        let target = target_dir(&write_root, out_ref);
        reject_existing_target(out_ref, &target)?;
        reject_namespace(out_ref, &write_root, &target)?;
        write_pack(out_ref, &target, &members)?;
        drop(self.scan());
        let Some(entry) = self.by_ref(out_ref)?.cloned() else {
            drop(std::fs::remove_dir_all(&target));
            drop(self.scan());
            return Err(PackError::with(
                PackCode::PackWriteFailed,
                format!(
                    "composed context pack '{out_ref}' was not discoverable at its durable ref"
                ),
                json!({"ref": out_ref, "targetDir": target.display().to_string()}),
            ));
        };
        if entry.source_path != target {
            drop(std::fs::remove_dir_all(&target));
            drop(self.scan());
            return Err(PackError::with(
                PackCode::PackWriteFailed,
                format!(
                    "composed context pack '{out_ref}' was not discoverable at its durable ref"
                ),
                json!({"ref": out_ref, "targetDir": target.display().to_string()}),
            ));
        }
        let plain_inputs: Vec<PlainInput<'_>> = members
            .iter()
            .map(|(path, _label, content)| PlainInput {
                path,
                content: Some(std::str::from_utf8(content).unwrap_or("")),
            })
            .collect();
        let assembled = assemble_plain(&plain_inputs);
        Ok(ComposeOutcome {
            refer: out_ref.to_owned(),
            entry,
            text: assembled.text,
            bytes: assembled.bytes,
            estimated_tokens: assembled.estimated_tokens,
            files: assembled.files,
        })
    }

    /// Delete a non-builtin pack and rescan.
    ///
    /// # Errors
    ///
    /// Returns [`PackError`] before any filesystem change when the ref is
    /// unsafe, absent, or builtin.
    pub fn remove_by_ref(&mut self, refer: &str) -> Result<RemoveOutcome, PackError> {
        assert_ref(refer)?;
        let Some(entry) = self.by_ref(refer)? else {
            return Err(PackError::with(
                PackCode::PackNotFound,
                format!("context pack ref '{refer}' not found in the library"),
                json!({"ref": refer}),
            ));
        };
        if entry.source_type == SourceType::Builtin {
            return Err(PackError::with(
                PackCode::PackNotRemovable,
                format!(
                    "context pack ref '{refer}' is a shipped builtin pack and cannot be removed"
                ),
                json!({"ref": refer, "sourceType": entry.source_type.as_str()}),
            ));
        }
        let removed_path = entry.source_path.clone();
        std::fs::remove_dir_all(&removed_path).map_err(|err| {
            PackError::new(
                PackCode::PackWriteFailed,
                format!("failed to remove context pack '{refer}': {err}"),
            )
        })?;
        drop(self.scan());
        Ok(RemoveOutcome {
            removed: true,
            refer: refer.to_owned(),
            removed_path,
        })
    }

    /// Resolve a manifest path inside the pack. A symlink that leaves the pack
    /// is refused.
    ///
    /// # Errors
    ///
    /// Returns [`PackError`] `file_outside_pack` for `..`, an absolute path, or
    /// a symlink whose target is outside the pack directory.
    pub fn resolve_file(entry: &PackEntry, rel: &str) -> Result<PathBuf, PackError> {
        if rel.contains("..") || rel.starts_with('/') || rel.starts_with('\\') {
            return Err(PackError::with(
                PackCode::FileOutsidePack,
                format!(
                    "relative path '{rel}' must be inside the pack directory (no '..', no leading '/')"
                ),
                json!({"packId": entry.id, "relPath": rel}),
            ));
        }
        let abs = contained(entry, rel)?;
        let base_display = entry.source_path.display().to_string();
        let abs_display = abs.display().to_string();
        let inside =
            abs_display.starts_with(&(base_display.clone() + "/")) || abs_display == base_display;
        if !inside {
            return Err(PackError::with(
                PackCode::FileOutsidePack,
                format!("resolved path '{abs_display}' falls outside pack '{base_display}'"),
                json!({"packId": entry.id, "relPath": rel, "resolved": abs_display}),
            ));
        }
        Ok(abs)
    }
}

fn contained(entry: &PackEntry, rel: &str) -> Result<PathBuf, PackError> {
    let mut cursor = entry.source_path.clone();
    for segment in rel.split(['/', '\\']) {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(PackError::with(
                PackCode::FileOutsidePack,
                format!(
                    "relative path '{rel}' must be inside the pack directory (no '..', no leading '/')"
                ),
                json!({"packId": entry.id, "relPath": rel}),
            ));
        }
        cursor.push(segment);
        if let Ok(meta) = std::fs::symlink_metadata(&cursor)
            && meta.file_type().is_symlink()
        {
            let canonical = std::fs::canonicalize(&cursor).map_err(|err| {
                PackError::new(
                    PackCode::FileReadFailed,
                    format!("failed to resolve symlink {}: {err}", cursor.display()),
                )
            })?;
            let base = std::fs::canonicalize(&entry.source_path)
                .unwrap_or_else(|_| entry.source_path.clone());
            if canonical.strip_prefix(&base).is_err() {
                return Err(PackError::with(
                    PackCode::FileOutsidePack,
                    format!(
                        "symlink '{}' escapes pack '{}'",
                        cursor.display(),
                        entry.source_path.display()
                    ),
                    json!({"packId": entry.id, "relPath": rel, "resolved": canonical.display().to_string()}),
                ));
            }
        }
    }
    Ok(cursor)
}

fn assert_ref(refer: &str) -> Result<(), PackError> {
    crewmux_spec::context::assert_safe_pack_ref(refer)
        .map_err(|message| PackError::with(PackCode::UnsafeRef, message, json!({"ref": refer})))
}

fn read_members(sources: &[ComposeFile]) -> Result<Vec<(String, String, Vec<u8>)>, PackError> {
    let mut members = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let meta =
            std::fs::metadata(&source.path).map_err(|err| read_failed(&source.path, &err))?;
        if !meta.is_file() {
            return Err(read_failed(
                &source.path,
                &std::io::Error::other("source is not a regular file"),
            ));
        }
        let content = std::fs::read(&source.path).map_err(|err| read_failed(&source.path, &err))?;
        let number = index.saturating_add(1);
        let path = format!("source-{number:04}{}", suffix_of(&source.label));
        members.push((path, source.label.clone(), content));
    }
    Ok(members)
}

fn read_failed(path: &Path, err: &std::io::Error) -> PackError {
    PackError::with(
        PackCode::FileReadFailed,
        format!(
            "failed to read composition source {}: {err}",
            path.display()
        ),
        json!({"path": path.display().to_string()}),
    )
}

fn suffix_of(label: &str) -> &'static str {
    let lower = label.to_ascii_lowercase();
    for suffix in [".markdown", ".yaml", ".yml", ".txt", ".md"] {
        if lower.ends_with(suffix) {
            return suffix;
        }
    }
    ".txt"
}

fn target_dir(root: &Path, refer: &str) -> PathBuf {
    refer
        .split('/')
        .fold(root.to_path_buf(), |path, segment| path.join(segment))
}

fn reject_existing_target(refer: &str, target: &Path) -> Result<(), PackError> {
    match std::fs::symlink_metadata(target) {
        Ok(_) => Err(PackError::with(
            PackCode::PackExists,
            format!("context pack target '{}' already exists", target.display()),
            json!({"ref": refer, "targetDir": target.display().to_string()}),
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotADirectory => Ok(()),
        Err(err) => Err(PackError::new(PackCode::PackWriteFailed, err.to_string())),
    }
}

fn reject_namespace(refer: &str, root: &Path, target: &Path) -> Result<(), PackError> {
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    let mut cursor = root.to_path_buf();
    if parent == root {
        return Ok(());
    }
    let rel = parent.strip_prefix(root).unwrap_or(parent);
    for segment in rel.components() {
        cursor.push(segment);
        let Ok(meta) = std::fs::symlink_metadata(&cursor) else {
            break;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(PackError::with(
                PackCode::UnsafeRefNamespace,
                format!(
                    "context pack ref '{refer}' crosses a symlink or non-directory namespace segment '{}'",
                    cursor.display()
                ),
                json!({"ref": refer, "segmentPath": cursor.display().to_string()}),
            ));
        }
        if cursor.join("manifest.yaml").is_file() {
            return Err(PackError::with(
                PackCode::PackRefBelowPack,
                format!(
                    "context pack ref '{refer}' is below existing pack leaf '{}'",
                    cursor.display()
                ),
                json!({"ref": refer, "ancestor": cursor.display().to_string()}),
            ));
        }
    }
    Ok(())
}

fn write_pack(
    refer: &str,
    target: &Path,
    members: &[(String, String, Vec<u8>)],
) -> Result<(), PackError> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| write_failed(refer, target, &err))?;
    }
    match std::fs::create_dir(target) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(PackError::with(
                PackCode::PackExists,
                format!("context pack target '{}' already exists", target.display()),
                json!({"ref": refer, "targetDir": target.display().to_string()}),
            ));
        }
        Err(err) => return Err(write_failed(refer, target, &err)),
    }
    let manifest = manifest_yaml(refer, members);
    let wrote = write_members(target, members).and_then(|()| {
        std::fs::write(target.join("manifest.yaml"), manifest)
            .map_err(|err| write_failed(refer, target, &err))
    });
    if let Err(err) = wrote {
        drop(std::fs::remove_dir_all(target));
        return Err(err);
    }
    Ok(())
}

fn write_members(target: &Path, members: &[(String, String, Vec<u8>)]) -> Result<(), PackError> {
    for (path, _label, content) in members {
        let dest = target.join(path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                PackError::new(
                    PackCode::PackWriteFailed,
                    format!("failed to write context pack: {err}"),
                )
            })?;
        }
        std::fs::write(&dest, content).map_err(|err| {
            PackError::new(
                PackCode::PackWriteFailed,
                format!("failed to write context pack: {err}"),
            )
        })?;
    }
    Ok(())
}

fn write_failed(refer: &str, target: &Path, err: &std::io::Error) -> PackError {
    PackError::with(
        PackCode::PackWriteFailed,
        format!("failed to write context pack '{refer}': {err}"),
        json!({"ref": refer, "targetDir": target.display().to_string()}),
    )
}

fn manifest_yaml(refer: &str, members: &[(String, String, Vec<u8>)]) -> String {
    let name = refer.rsplit('/').next().unwrap_or(refer);
    let mut out = format!(
        "name: {}\nversion: \"1\"\npurpose: {}\ntaxonomy: mission\nfiles:\n",
        yaml_quote(name),
        yaml_quote(&format!("Composed from {} ordered files", members.len()))
    );
    for (path, label, _content) in members {
        out.push_str("  - path: ");
        out.push_str(&yaml_quote(path));
        out.push_str("\n    role: source\n    summary: ");
        out.push_str(&yaml_quote(label));
        out.push('\n');
    }
    out
}

fn yaml_quote(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
