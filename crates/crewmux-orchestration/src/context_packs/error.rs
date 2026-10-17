//! Structured context-pack failures. Codes match `ContextPackError`.

use serde_json::{Value, json};
use thiserror::Error;

/// Machine code for a pack failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackCode {
    /// Manifest missing.
    ManifestMissing,
    /// YAML parse failure.
    ManifestParse,
    /// Schema failure.
    ManifestInvalid,
    /// Safe ref, no pack.
    PackNotFound,
    /// Path left the pack.
    FileOutsidePack,
    /// Read failed.
    FileReadFailed,
    /// Ref failed the segment contract.
    UnsafeRef,
    /// Compose had no sources, or a source was missing.
    MissingFiles,
    /// The ref or target already exists.
    PackExists,
    /// Ref is under a manifest leaf.
    PackRefBelowPack,
    /// A namespace segment is a symlink or not a directory.
    UnsafeRefNamespace,
    /// A write failed.
    PackWriteFailed,
    /// No `user_file` root.
    StoreUnavailable,
    /// Builtin packs are not removable.
    PackNotRemovable,
}

impl PackCode {
    /// Stable wire code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManifestMissing => "manifest_missing",
            Self::ManifestParse => "manifest_parse_error",
            Self::ManifestInvalid => "manifest_invalid",
            Self::PackNotFound => "pack_not_found",
            Self::FileOutsidePack => "file_outside_pack",
            Self::FileReadFailed => "file_read_failed",
            Self::UnsafeRef => "unsafe_ref",
            Self::MissingFiles => "missing_files",
            Self::PackExists => "pack_exists",
            Self::PackRefBelowPack => "pack_ref_below_pack",
            Self::UnsafeRefNamespace => "unsafe_ref_namespace",
            Self::PackWriteFailed => "pack_write_failed",
            Self::StoreUnavailable => "store_unavailable",
            Self::PackNotRemovable => "pack_not_removable",
        }
    }

    /// HTTP status used by the context routes.
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::UnsafeRef
            | Self::MissingFiles
            | Self::PackRefBelowPack
            | Self::UnsafeRefNamespace
            | Self::ManifestMissing
            | Self::ManifestParse
            | Self::ManifestInvalid
            | Self::FileOutsidePack => 400,
            Self::PackNotRemovable => 403,
            Self::PackNotFound => 404,
            Self::PackExists => 409,
            Self::PackWriteFailed | Self::FileReadFailed => 500,
            Self::StoreUnavailable => 503,
        }
    }
}

/// A context-pack operation failure.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct PackError {
    /// Machine code.
    pub code: PackCode,
    /// TypeScript sentence.
    pub message: String,
    /// Extra JSON fields.
    pub details: Value,
}

impl PackError {
    /// Wire object: `error`, `message`, and any extra detail fields.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut body = json!({"error": self.code.as_str(), "message": self.message});
        if let (Some(object), Some(extra)) = (body.as_object_mut(), self.details.as_object()) {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        body
    }

    /// A failure with an empty detail object.
    #[must_use]
    pub fn new(code: PackCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: json!({}),
        }
    }

    pub(crate) fn with(code: PackCode, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }
}
