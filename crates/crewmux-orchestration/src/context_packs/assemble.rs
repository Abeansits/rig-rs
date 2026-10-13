// Derived from openrig packages/daemon/src/domain/context-packs/bundle-assembler.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Plain concatenation and the preview frame.

use super::entry::PackEntry;
use super::error::{PackCode, PackError};
use super::tokens::estimate_tokens_from_bytes;

/// Separator between present members. Source bytes are otherwise untouched.
pub(crate) const PLAIN_SEPARATOR: &str = "\n\n";

/// One plain member. `None` content is a missing file.
pub struct PlainInput<'a> {
    /// Member path.
    pub path: &'a str,
    /// Bytes, or missing.
    pub content: Option<&'a str>,
}

/// Plain assembly.
pub struct PlainAssembly {
    /// Joined present contents.
    pub text: String,
    /// UTF-8 length.
    pub bytes: u64,
    /// Token estimate.
    pub estimated_tokens: u32,
    /// Present members.
    pub files: Vec<PlainFileStat>,
    /// Missing member paths.
    pub missing: Vec<String>,
}

/// One present member's size.
pub struct PlainFileStat {
    /// Path.
    pub path: String,
    /// UTF-8 length.
    pub bytes: u64,
    /// Token estimate.
    pub estimated_tokens: u32,
}

/// Concatenate present contents in order.
#[must_use]
pub fn assemble_plain(files: &[PlainInput<'_>]) -> PlainAssembly {
    let mut text = String::new();
    let mut stats = Vec::new();
    let mut missing = Vec::new();
    for file in files {
        let Some(content) = file.content else {
            missing.push(file.path.to_owned());
            continue;
        };
        if !stats.is_empty() {
            text.push_str(PLAIN_SEPARATOR);
        }
        text.push_str(content);
        let bytes = u64::try_from(content.len()).unwrap_or(u64::MAX);
        stats.push(PlainFileStat {
            path: file.path.to_owned(),
            bytes,
            estimated_tokens: estimate_tokens_from_bytes(bytes),
        });
    }
    let bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    PlainAssembly {
        estimated_tokens: estimate_tokens_from_bytes(bytes),
        bytes,
        text,
        files: stats,
        missing,
    }
}

/// Preview bundle.
pub struct Bundle {
    /// Framed text, trailing newline included.
    pub text: String,
    /// UTF-8 length of `text`.
    pub bytes: u64,
    /// Token estimate of `text`.
    pub estimated_tokens: u32,
    /// Present files, sized from the raw content.
    pub files: Vec<BundleFile>,
    /// Missing files.
    pub missing: Vec<(String, String)>,
}

/// One present preview file.
pub struct BundleFile {
    /// Path.
    pub path: String,
    /// Role.
    pub role: String,
    /// Raw content bytes.
    pub bytes: u64,
    /// Token estimate.
    pub estimated_tokens: u32,
}

/// Frame a pack the way preview does. Missing files are warnings.
///
/// # Errors
///
/// Returns [`PackError`] when a present file cannot be read.
pub fn assemble_bundle(entry: &PackEntry) -> Result<Bundle, PackError> {
    let mut sections = vec![format!(
        "# OpenRig Context Pack: {} v{}",
        entry.name, entry.version
    )];
    if let Some(purpose) = entry.purpose.as_deref() {
        let trimmed = purpose.trim();
        if !trimmed.is_empty() {
            sections.push(trimmed.to_owned());
        }
    }
    let mut files = Vec::new();
    let mut missing = Vec::new();
    for file in &entry.files {
        let Some(path) = &file.absolute_path else {
            missing.push((file.path.clone(), file.role.clone()));
            continue;
        };
        let content = std::fs::read_to_string(path).map_err(|err| {
            PackError::with(
                PackCode::FileReadFailed,
                format!("failed to read pack file {}: {err}", path.display()),
                serde_json::json!({"packId": entry.id, "path": file.path}),
            )
        })?;
        let header = match &file.summary {
            Some(summary) => format!("## File: {} (role: {}) — {summary}", file.path, file.role),
            None => format!("## File: {} (role: {})", file.path, file.role),
        };
        sections.push(header);
        sections.push(trim_end(&content));
        let bytes = u64::try_from(content.len()).unwrap_or(u64::MAX);
        files.push(BundleFile {
            path: file.path.clone(),
            role: file.role.clone(),
            bytes,
            estimated_tokens: estimate_tokens_from_bytes(bytes),
        });
    }
    let mut text = sections.join("\n\n");
    text.push('\n');
    let bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    Ok(Bundle {
        estimated_tokens: estimate_tokens_from_bytes(bytes),
        bytes,
        text,
        files,
        missing,
    })
}

fn trim_end(text: &str) -> String {
    text.trim_end_matches([' ', '\t', '\n', '\r']).to_owned()
}
