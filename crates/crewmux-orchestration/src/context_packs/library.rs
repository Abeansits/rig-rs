// Derived from openrig packages/daemon/src/domain/context-packs/context-pack-library-service.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Discovery. Packs are leaves. The same physical directory is claimed once.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crewmux_spec::context::{assert_safe_pack_ref, parse_manifest};

use super::entry::{EntryFile, PackEntry, SourceType, context_pack_id};
use super::error::{PackCode, PackError};
use super::time::iso_from_system;
use super::tokens::estimate_tokens_from_bytes;

/// One discovery root.
#[derive(Clone, Debug)]
pub struct Root {
    /// Absolute or caller-supplied path.
    pub path: PathBuf,
    /// How packs found here are labeled.
    pub source_type: SourceType,
}

impl Root {
    /// A root.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, source_type: SourceType) -> Self {
        Self {
            path: path.into(),
            source_type,
        }
    }
}

/// Result of one scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncReport {
    /// Indexed packs.
    pub count: usize,
    /// Packs that were skipped, with the sentence.
    pub errors: Vec<ScanError>,
}

/// One scan failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanError {
    /// Pack directory.
    pub source: String,
    /// Sentence.
    pub error: String,
}

/// In-memory pack index.
#[derive(Clone, Debug)]
pub struct Library {
    roots: Vec<Root>,
    entries: BTreeMap<String, PackEntry>,
}

impl Library {
    /// Empty index. Call [`scan`](Self::scan) before reads.
    #[must_use]
    pub fn new(roots: Vec<Root>) -> Self {
        Self {
            roots,
            entries: BTreeMap::new(),
        }
    }

    /// Configured roots, in precedence order. Last wins on the same ref.
    #[must_use]
    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// Re-walk every root.
    #[must_use]
    pub fn scan(&mut self) -> SyncReport {
        let mut next = BTreeMap::new();
        let mut claimed = BTreeSet::new();
        let mut errors = Vec::new();
        for root in &self.roots {
            for (pack_dir, refer) in discover(&root.path) {
                let physical =
                    std::fs::canonicalize(&pack_dir).unwrap_or_else(|_| pack_dir.clone());
                if !claimed.insert(physical) {
                    continue;
                }
                if let Err(err) = assert_safe_pack_ref(&refer) {
                    errors.push(ScanError {
                        source: pack_dir.display().to_string(),
                        error: err,
                    });
                    continue;
                }
                match read_entry(&pack_dir, root, &refer) {
                    Ok(entry) => {
                        next.insert(refer, entry);
                    }
                    Err(err) => errors.push(ScanError {
                        source: pack_dir.display().to_string(),
                        error: format!("{}: {}", err.code.as_str(), err.message),
                    }),
                }
            }
        }
        let count = next.len();
        self.entries = next;
        SyncReport { count, errors }
    }

    /// Resolve a ref. An unsafe ref is an error. A missing safe ref is `None`.
    ///
    /// # Errors
    ///
    /// Returns [`PackError`] with `unsafe_ref` before any lookup.
    pub fn by_ref(&self, refer: &str) -> Result<Option<&PackEntry>, PackError> {
        assert_ref(refer)?;
        Ok(self.entries.get(refer))
    }

    /// Indexed packs, sorted by name, version, then ref.
    #[must_use]
    pub fn list(&self) -> Vec<&PackEntry> {
        let mut entries: Vec<&PackEntry> = self.entries.values().collect();
        entries.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.version.cmp(&right.version))
                .then(left.relative_path.cmp(&right.relative_path))
        });
        entries
    }

    pub(crate) fn contains(&self, refer: &str) -> bool {
        self.entries.contains_key(refer)
    }
}

fn assert_ref(refer: &str) -> Result<(), PackError> {
    assert_safe_pack_ref(refer).map_err(|message| {
        PackError::with(
            PackCode::UnsafeRef,
            message,
            serde_json::json!({"ref": refer}),
        )
    })
}

fn discover(root: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    if root.is_dir() {
        walk(root, root, &mut found);
    }
    found
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(".tmp-add-") {
            continue;
        }
        let child = entry.path();
        if child.join("manifest.yaml").is_file() {
            let refer = child
                .strip_prefix(root)
                .map(|rel| {
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_default();
            found.push((child, refer));
        } else {
            walk(root, &child, found);
        }
    }
}

fn read_entry(pack_dir: &Path, root: &Root, refer: &str) -> Result<PackEntry, PackError> {
    let manifest_path = pack_dir.join("manifest.yaml");
    let raw = std::fs::read_to_string(&manifest_path).map_err(|err| {
        PackError::new(
            PackCode::ManifestMissing,
            format!(
                "manifest at {} is unreadable: {err}",
                manifest_path.display()
            ),
        )
    })?;
    let manifest = parse_manifest(&raw, &manifest_path.display().to_string()).map_err(|err| {
        let code = match err.code {
            crewmux_spec::context::ManifestCode::Parse => PackCode::ManifestParse,
            crewmux_spec::context::ManifestCode::Invalid => PackCode::ManifestInvalid,
        };
        PackError::new(code, err.message)
    })?;
    let mut newest = std::fs::metadata(&manifest_path)
        .and_then(|meta| meta.modified())
        .ok();
    let mut files = Vec::with_capacity(manifest.files.len());
    for file in &manifest.files {
        let abs = pack_dir.join(&file.path);
        let meta = std::fs::metadata(&abs).ok();
        let bytes = meta.as_ref().map(std::fs::Metadata::len);
        if let Some(modified) = meta.as_ref().and_then(|meta| meta.modified().ok())
            && newest.is_none_or(|current| modified > current)
        {
            newest = Some(modified);
        }
        let estimated = bytes.map(estimate_tokens_from_bytes);
        files.push(EntryFile {
            path: file.path.clone(),
            role: file.role.clone(),
            summary: file.summary.clone(),
            absolute_path: bytes.map(|_| abs),
            bytes,
            estimated_tokens: estimated,
        });
    }
    let derived = files.iter().fold(0u32, |sum, file| {
        sum.saturating_add(file.estimated_tokens.unwrap_or(0))
    });
    let updated_at = newest.map_or_else(
        || iso_from_system(std::time::SystemTime::now()),
        iso_from_system,
    );
    Ok(PackEntry {
        id: context_pack_id(refer),
        name: manifest.name,
        version: manifest.version,
        purpose: manifest.description,
        taxonomy: manifest.taxonomy.as_str().to_owned(),
        source_type: root.source_type,
        source_path: pack_dir.to_path_buf(),
        relative_path: refer.to_owned(),
        updated_at,
        manifest_estimated_tokens: manifest.estimated_tokens,
        derived_estimated_tokens: derived,
        files,
    })
}
