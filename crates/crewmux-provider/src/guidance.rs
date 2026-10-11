// Derived from openrig packages/daemon/src/domain/managed-blocks.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Managed regions in guidance files. Marker text matches the upstream file format
//! so an existing block can be replaced literally, including `$` sequences.

use std::collections::BTreeMap;

/// In-memory files used by the guidance tests and by callers that do not touch disk.
#[derive(Debug, Default)]
pub struct MemoryFs {
    /// Path to contents.
    pub files: BTreeMap<String, String>,
}

impl MemoryFs {
    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    fn read(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    fn write(&mut self, path: &str, content: String) {
        self.files.insert(path.to_owned(), content);
    }
}

/// Insert or replace the managed block `id` with `content` as literal text.
pub fn merge_managed_block(fs: &mut MemoryFs, target: &str, id: &str, content: &str) {
    let begin = block_start(id);
    let end = block_end(id);
    let block = format!("{begin}\n{content}\n{end}");
    if !fs.exists(target) {
        fs.write(target, block);
        return;
    }
    let existing = fs.read(target).unwrap_or("").to_owned();
    if let Some(updated) = replace_block(&existing, id, &block) {
        fs.write(target, format!("{updated}\n"));
        return;
    }
    fs.write(target, format!("{existing}\n\n{block}"));
}

fn replace_block(existing: &str, id: &str, block: &str) -> Option<String> {
    let current = (block_start(id), block_end(id));
    let legacy = (legacy_start(id), legacy_end(id));
    let mut updated = existing.to_owned();
    let mut replaced = false;
    for (begin, end) in [current, legacy] {
        if let Some(next) = replace_span(&updated, &begin, &end, block) {
            updated = next;
            replaced = true;
        }
    }
    replaced.then_some(updated)
}

fn replace_span(file: &str, begin: &str, end: &str, block: &str) -> Option<String> {
    let start = file.find(begin)?;
    let relative = file.get(start..)?.find(end)?;
    let stop = start.saturating_add(relative).saturating_add(end.len());
    let mut out = String::new();
    out.push_str(file.get(..start)?);
    out.push_str(block);
    out.push_str(file.get(stop..).unwrap_or(""));
    Some(out)
}

fn block_start(id: &str) -> String {
    format!("<!-- BEGIN OpenRig MANAGED BLOCK: {id} -->")
}

fn block_end(id: &str) -> String {
    format!("<!-- END OpenRig MANAGED BLOCK: {id} -->")
}

fn legacy_start(id: &str) -> String {
    format!("<!-- BEGIN RIGGED MANAGED BLOCK: {id} -->")
}

fn legacy_end(id: &str) -> String {
    format!("<!-- END RIGGED MANAGED BLOCK: {id} -->")
}
