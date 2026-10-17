//! Library entry projected the way the TypeScript routes do.

use std::path::PathBuf;

use serde_json::{Value, json};

/// Where a pack was discovered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceType {
    /// Shipped with the binary.
    Builtin,
    /// `$RIG_HOME/context`.
    UserFile,
    /// A workspace pack.
    Workspace,
}

impl SourceType {
    /// Wire literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::UserFile => "user_file",
            Self::Workspace => "workspace",
        }
    }
}

/// One declared file plus what the scan found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryFile {
    /// Manifest path.
    pub path: String,
    /// Role.
    pub role: String,
    /// Summary, when the manifest has one.
    pub summary: Option<String>,
    /// Absolute path when the file exists.
    pub absolute_path: Option<PathBuf>,
    /// Size when the file exists.
    pub bytes: Option<u64>,
    /// Per-file estimate.
    pub estimated_tokens: Option<u32>,
}

/// One indexed pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackEntry {
    /// `context-pack:<ref>`.
    pub id: String,
    /// Manifest name.
    pub name: String,
    /// Manifest version.
    pub version: String,
    /// Manifest purpose.
    pub purpose: Option<String>,
    /// Taxonomy literal.
    pub taxonomy: String,
    /// Discovery root kind.
    pub source_type: SourceType,
    /// Pack directory.
    pub source_path: PathBuf,
    /// Path-like ref.
    pub relative_path: String,
    /// Newest mtime in the pack, ISO-8601.
    pub updated_at: String,
    /// Operator estimate.
    pub manifest_estimated_tokens: Option<u32>,
    /// Sum of file estimates.
    pub derived_estimated_tokens: u32,
    /// Declared files.
    pub files: Vec<EntryFile>,
}

impl PackEntry {
    /// Route JSON. Nulls are present, matching the TypeScript wire.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": "context-pack",
            "name": self.name,
            "version": self.version,
            "purpose": self.purpose,
            "taxonomy": self.taxonomy,
            "sourceType": self.source_type.as_str(),
            "sourcePath": self.source_path.display().to_string(),
            "relativePath": self.relative_path,
            "updatedAt": self.updated_at,
            "manifestEstimatedTokens": self.manifest_estimated_tokens,
            "derivedEstimatedTokens": self.derived_estimated_tokens,
            "files": self.files.iter().map(EntryFile::to_json).collect::<Vec<_>>(),
        })
    }
}

impl EntryFile {
    fn to_json(&self) -> Value {
        json!({
            "path": self.path,
            "role": self.role,
            "summary": self.summary,
            "absolutePath": self.absolute_path.as_ref().map(|path| path.display().to_string()),
            "bytes": self.bytes,
            "estimatedTokens": self.estimated_tokens,
        })
    }
}

/// Opaque id. The ref is the identity.
#[must_use]
pub fn context_pack_id(refer: &str) -> String {
    format!("context-pack:{refer}")
}
