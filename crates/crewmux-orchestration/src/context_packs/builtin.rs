// Derived from openrig scripts/generate-context-packs.mjs
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Shipped packs, generated into `builtin_gen.rs` by `cargo xtask generate-context-packs`.

#[path = "builtin_gen.rs"]
mod builtin_gen;

use std::path::Path;

/// Copy the embedded packs into `dest` when the bytes differ.
///
/// # Errors
///
/// Returns an error when a directory or file cannot be written.
pub fn materialize_builtin(dest: &Path) -> std::io::Result<()> {
    for (rel, bytes) in builtin_gen::FILES {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::read(&path).ok().as_deref() == Some(*bytes) {
            continue;
        }
        std::fs::write(path, bytes)?;
    }
    Ok(())
}
