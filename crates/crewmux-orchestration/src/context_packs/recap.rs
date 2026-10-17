// Derived from openrig packages/daemon/src/domain/context-packs/seat-recap-store.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Seat-homed recaps. Addressability is the only write gate.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_spec::context::{AddressFinding, parse_sections, validate_addressability};

/// Current recap filename.
pub const RECAP_FILENAME: &str = "RECAP.md";
const CHAIN_DIRNAME: &str = "recap-superseded";

/// Addressability refusal, or a publication failure that left the current recap.
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

/// Injected publication fault. Production leaves this unset.
#[derive(Clone, Copy, Debug)]
pub struct RecapFault {
    /// `write`, `archive`, `link`, `publish`, `fallback-copy`, or `source`.
    pub phase: &'static str,
    /// Raw errno that makes `hard_link` take the exclusive-copy fallback.
    pub link_code: Option<i32>,
}

thread_local! {
    static FAULT: Cell<Option<RecapFault>> = const { Cell::new(None) };
}

/// Install a durability fault for the calling thread.
pub fn set_recap_fault_for_tests(fault: Option<RecapFault>) {
    FAULT.with(|cell| cell.set(fault));
}

fn fault() -> Option<RecapFault> {
    FAULT.with(Cell::get)
}

/// A resolved authored recap, or a labeled absence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecapPointer {
    /// The seat directory has `RECAP.md`.
    Resolved {
        /// Always `seat:RECAP.md`.
        address: String,
        /// Superseded files under that seat.
        chain_length: usize,
    },
    /// The ref did not resolve. The reason names what was tried.
    Absent {
        /// Labeled sentence.
        reason: String,
    },
}

/// Write `RECAP.md`, archiving the previous file under `recap-superseded/`.
///
/// The new bytes are staged first. The previous current is hard-linked into the
/// chain, or copied exclusively when the link fails with `EOPNOTSUPP`,
/// `ENOTSUP`, `EPERM`, or `EXDEV`. A failed attempt removes files it created
/// and leaves the current recap in place.
///
/// # Errors
///
/// Returns [`RecapError`] when the markdown is not addressable, before any
/// recap byte is replaced, or when publication fails after cleanup.
pub fn write_seat_recap(
    seat_dir: &Path,
    content: &str,
    now_ms: u64,
) -> Result<Vec<ContractFinding>, RecapError> {
    let findings = validate_addressability(content);
    if !findings.is_empty() {
        return Err(RecapError {
            message: format!(
                "recap is not addressable and could never compose (seat:{RECAP_FILENAME}#... would fail every \
                 handover profile): {}. Fix the structure (duplicate header paths / unterminated fences); \
                 prose shape is never gated.",
                findings
                    .iter()
                    .map(finding_text)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        });
    }
    let advisory = validate_recap_contract(content);
    std::fs::create_dir_all(seat_dir).map_err(|err| io_err(&err))?;
    publish(seat_dir, content, now_ms)?;
    Ok(advisory)
}

fn publish(seat_dir: &Path, content: &str, now_ms: u64) -> Result<(), RecapError> {
    let current = seat_dir.join(RECAP_FILENAME);
    let staged = seat_dir.join(format!(".RECAP-{}.tmp", unique_token()));
    let mut archive_stage: Option<PathBuf> = None;
    let mut archived: Option<PathBuf> = None;
    let mut published = false;
    let result = publish_attempt(&mut Attempt {
        seat_dir,
        content,
        now_ms,
        current: &current,
        staged: &staged,
        archive_stage: &mut archive_stage,
        archived: &mut archived,
        published: &mut published,
    });
    cleanup(
        &staged,
        archive_stage.as_deref(),
        archived.as_deref(),
        published,
    );
    result
}

struct Attempt<'a> {
    seat_dir: &'a Path,
    content: &'a str,
    now_ms: u64,
    current: &'a Path,
    staged: &'a Path,
    archive_stage: &'a mut Option<PathBuf>,
    archived: &'a mut Option<PathBuf>,
    published: &'a mut bool,
}

fn publish_attempt(job: &mut Attempt<'_>) -> Result<(), RecapError> {
    exclusive_create(job.staged)?;
    write_staged(job.staged, job.content)?;
    if job.current.is_file() {
        let chain = job.seat_dir.join(CHAIN_DIRNAME);
        std::fs::create_dir_all(&chain).map_err(|err| io_err(&err))?;
        let stage = chain.join(format!(".RECAP-{}.tmp", unique_token()));
        exclusive_create(&stage)?;
        *job.archive_stage = Some(stage.clone());
        copy_source(job.current, &stage)?;
        *job.archived = Some(link_archive(&chain, &stage, job.now_ms, job.archived)?);
    }
    rename_current(job.staged, job.current)?;
    *job.published = true;
    Ok(())
}

fn write_staged(staged: &Path, content: &str) -> Result<(), RecapError> {
    if fault().is_some_and(|item| item.phase == "write") && content.contains("new recap") {
        drop(std::fs::write(staged, "partial new recap"));
        return Err(RecapError {
            message: "injected partial write".to_owned(),
        });
    }
    std::fs::write(staged, content).map_err(|err| io_err(&err))
}

fn copy_source(current: &Path, stage: &Path) -> Result<(), RecapError> {
    match fault().map(|item| item.phase) {
        Some("source") => {
            return Err(RecapError {
                message: "injected unreadable archive source".to_owned(),
            });
        }
        Some("archive") => {
            drop(std::fs::write(stage, "partial archive"));
            return Err(RecapError {
                message: "injected archive copy".to_owned(),
            });
        }
        _ => {}
    }
    std::fs::copy(current, stage)
        .map(|_| ())
        .map_err(|err| io_err(&err))
}

fn link_archive(
    chain: &Path,
    stage: &Path,
    now_ms: u64,
    owned: &mut Option<PathBuf>,
) -> Result<PathBuf, RecapError> {
    let stamp = format!("{now_ms:015}");
    for counter in 1..10_000 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{counter}")
        };
        let target = chain.join(format!("RECAP-{stamp}{suffix}.md"));
        match place_archive(stage, &target, owned) {
            Ok(()) => return Ok(target),
            Err(Place::Exists) => {}
            Err(Place::Failed(err)) => return Err(err),
        }
    }
    Err(RecapError {
        message: "recap chain could not reserve a superseded name".to_owned(),
    })
}

enum Place {
    Exists,
    Failed(RecapError),
}

fn place_archive(stage: &Path, target: &Path, owned: &mut Option<PathBuf>) -> Result<(), Place> {
    match try_link(stage, target) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(Place::Exists),
        Err(err) if link_fallback(&err) => copy_exclusive(stage, target, owned),
        Err(err) => Err(Place::Failed(io_err(&err))),
    }
}

fn copy_exclusive(stage: &Path, target: &Path, owned: &mut Option<PathBuf>) -> Result<(), Place> {
    match exclusive_create(target) {
        Err(err) if err.message.contains("File exists") || already_message(&err) => {
            return Err(Place::Exists);
        }
        Err(err) => return Err(Place::Failed(err)),
        Ok(()) => {}
    }
    *owned = Some(target.to_path_buf());
    if fault().is_some_and(|item| item.phase == "fallback-copy") {
        drop(std::fs::write(target, "partial archive"));
        return Err(Place::Failed(RecapError {
            message: "injected fallback copy".to_owned(),
        }));
    }
    std::fs::copy(stage, target)
        .map(|_| ())
        .map_err(|err| Place::Failed(io_err(&err)))
}

fn already_message(err: &RecapError) -> bool {
    err.message.contains("AlreadyExists") || err.message.contains("File exists")
}

fn try_link(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(code) = fault().and_then(|item| item.link_code) {
        return Err(std::io::Error::from_raw_os_error(code));
    }
    if fault().is_some_and(|item| item.phase == "link") {
        return Err(std::io::Error::other(
            "injected archive publication failure",
        ));
    }
    std::fs::hard_link(from, to)
}

fn link_fallback(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(1 | 18 | 95)) || err.kind() == std::io::ErrorKind::Unsupported
}

fn rename_current(staged: &Path, current: &Path) -> Result<(), RecapError> {
    if fault().is_some_and(|item| item.phase == "publish") {
        return Err(RecapError {
            message: "injected publication failure".to_owned(),
        });
    }
    std::fs::rename(staged, current).map_err(|err| io_err(&err))
}

fn exclusive_create(path: &Path) -> Result<(), RecapError> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
        .map_err(|err| io_err(&err))
}

fn cleanup(staged: &Path, archive_stage: Option<&Path>, archived: Option<&Path>, published: bool) {
    drop(std::fs::remove_file(staged));
    if let Some(path) = archive_stage {
        drop(std::fs::remove_file(path));
    }
    if !published && let Some(path) = archived {
        drop(std::fs::remove_file(path));
    }
}

fn unique_token() -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
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

/// Superseded recaps, oldest first. Names that are not chain files are ignored.
#[must_use]
pub fn list_recap_chain(seat_dir: &Path) -> Vec<PathBuf> {
    let chain = seat_dir.join(CHAIN_DIRNAME);
    let Ok(entries) = std::fs::read_dir(chain) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(key) = chain_key(name) else {
            continue;
        };
        rows.push((key, entry.path()));
    }
    rows.sort_by_key(|row| row.0);
    rows.into_iter().map(|(_, path)| path).collect()
}

fn chain_key(name: &str) -> Option<(u64, u64)> {
    let rest = name.strip_prefix("RECAP-")?.strip_suffix(".md")?;
    let (stamp, sequence) = match rest.split_once('-') {
        Some((stamp, sequence)) => (stamp, sequence.parse::<u64>().ok()?),
        None => (rest, 1),
    };
    Some((stamp.parse::<u64>().ok()?, sequence))
}

/// Resolve `seat@rig` under `topology_root/rigs/<rig>/seats/<seat>`.
///
/// The split is the first `@`. A host-qualified rig keeps the rest (`s1@r1@host`
/// is seat `s1`, rig `r1@host`). Anything else is a labeled absence.
#[must_use]
pub fn resolve_authored_recap_pointer(seat_ref: &str, topology_root: &Path) -> RecapPointer {
    let Some((member, rig)) = canonical_seat(seat_ref) else {
        let kind = if seat_ref.contains('@') {
            "malformed"
        } else if legacy_flat(seat_ref) {
            "legacy"
        } else {
            "malformed"
        };
        return RecapPointer::Absent {
            reason: format!(
                "seat ref '{seat_ref}' did not parse as canonical <seat>@<rig> (parse verdict: {kind}) — authored recap not resolved, never guessed"
            ),
        };
    };
    let seat_dir = topology_root
        .join("rigs")
        .join(rig)
        .join("seats")
        .join(member);
    if !seat_dir.join(RECAP_FILENAME).is_file() {
        return RecapPointer::Absent {
            reason: format!(
                "no {RECAP_FILENAME} on the seat tree ({}) — the predecessor never wrote one",
                seat_dir.display()
            ),
        };
    }
    RecapPointer::Resolved {
        address: format!("seat:{RECAP_FILENAME}"),
        chain_length: list_recap_chain(&seat_dir).len(),
    }
}

fn canonical_seat(raw: &str) -> Option<(&str, &str)> {
    let (member, rig) = raw.split_once('@')?;
    if member.is_empty() || rig.is_empty() {
        return None;
    }
    Some((member, rig))
}

fn legacy_flat(raw: &str) -> bool {
    let Some(rest) = raw.strip_prefix('r') else {
        return false;
    };
    let Some((digits, tail)) = rest.split_once('-') else {
        return false;
    };
    digits.len() == 2 && digits.bytes().all(|byte| byte.is_ascii_digit()) && !tail.is_empty()
}
