// Derived from openrig packages/daemon/src/domain/context-packs/seat-recap-store.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Seat-homed recaps. Addressability is the only write gate.

use std::io::Write;
use std::path::{Path, PathBuf};

use crewmux_spec::context::{AddressFinding, parse_sections, validate_addressability};

/// Current recap filename.
pub const RECAP_FILENAME: &str = "RECAP.md";
const CHAIN_DIRNAME: &str = "recap-superseded";

/// Addressability refusal. Nothing was written.
#[derive(Debug)]
pub struct RecapError {
    /// TypeScript sentence.
    pub message: String,
}

impl std::fmt::Display for RecapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RecapError {}

/// Advisory authoring findings. They never block the write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractFinding {
    /// No section slug contains `decision`.
    NoDecisionsSection,
    /// A line says unverified without the canonical marker.
    NonstandardUnverified {
        /// Zero-based line.
        line: usize,
    },
}

/// Write `RECAP.md`, archiving the previous file under `recap-superseded/`.
///
/// # Errors
///
/// Returns [`RecapError`] when the markdown is not addressable, before any
/// recap byte is replaced.
pub fn write_seat_recap(
    seat_dir: &Path,
    content: &str,
    now_ms: u64,
) -> Result<Vec<ContractFinding>, RecapError> {
    let findings = validate_addressability(content);
    if !findings.is_empty() {
        let detail = findings
            .iter()
            .map(finding_text)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(RecapError {
            message: format!(
                "recap is not addressable and could never compose (seat:{RECAP_FILENAME}#... would fail every \
                 handover profile): {detail}. Fix the structure (duplicate header paths / unterminated fences); \
                 prose shape is never gated."
            ),
        });
    }
    let advisory = validate_recap_contract(content);
    std::fs::create_dir_all(seat_dir).map_err(|err| io_err(&err))?;
    let current = seat_dir.join(RECAP_FILENAME);
    if current.is_file() {
        archive_current(seat_dir, &current, now_ms)?;
    }
    write_new(&current, content)?;
    Ok(advisory)
}

/// Checkable authoring contract. Empty means nothing to flag.
#[must_use]
pub fn validate_recap_contract(content: &str) -> Vec<ContractFinding> {
    let mut findings = Vec::new();
    let sections = parse_sections(content);
    let has_decisions = sections.iter().any(|section| {
        section
            .header_path
            .last()
            .is_some_and(|slug| slug.contains("decision"))
    });
    if !has_decisions {
        findings.push(ContractFinding::NoDecisionsSection);
    }
    for (index, line) in content.split('\n').enumerate() {
        if line.to_ascii_lowercase().contains("unverified") && !line.contains("UNVERIFIED:") {
            findings.push(ContractFinding::NonstandardUnverified { line: index });
        }
    }
    findings
}

fn archive_current(seat_dir: &Path, current: &Path, now_ms: u64) -> Result<(), RecapError> {
    let chain = seat_dir.join(CHAIN_DIRNAME);
    std::fs::create_dir_all(&chain).map_err(|err| io_err(&err))?;
    let bytes = std::fs::read(current).map_err(|err| io_err(&err))?;
    let stamp = format!("{now_ms:015}");
    for counter in 1..10_000 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{counter}")
        };
        let target = chain.join(format!("RECAP-{stamp}{suffix}.md"));
        match exclusive_write(&target, &bytes) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(io_err(&err)),
        }
    }
    Err(RecapError {
        message: "recap chain could not reserve a superseded name".to_owned(),
    })
}

fn exclusive_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn write_new(current: &Path, content: &str) -> Result<(), RecapError> {
    let staged = current.with_file_name(".RECAP.write.tmp");
    std::fs::write(&staged, content).map_err(|err| io_err(&err))?;
    std::fs::rename(&staged, current).map_err(|err| {
        drop(std::fs::remove_file(&staged));
        io_err(&err)
    })
}

fn finding_text(finding: &AddressFinding) -> String {
    match finding {
        AddressFinding::UnterminatedFence { line } => format!("unterminated fence at line {line}"),
        AddressFinding::Duplicate { header_path, .. } => {
            format!("duplicate-header-path '{header_path}'")
        }
        AddressFinding::Unaddressable { header_path, .. } => {
            format!("unaddressable-header '{header_path}'")
        }
    }
}

fn io_err(err: &std::io::Error) -> RecapError {
    RecapError {
        message: err.to_string(),
    }
}

/// Superseded recaps, oldest first.
#[must_use]
pub fn list_recap_chain(seat_dir: &Path) -> Vec<PathBuf> {
    let chain = seat_dir.join(CHAIN_DIRNAME);
    let Ok(entries) = std::fs::read_dir(chain) else {
        return Vec::new();
    };
    let mut names: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    names.sort();
    names
}
