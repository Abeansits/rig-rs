//! Thread ids from `$CODEX_HOME/sessions/**/rollout-*.jsonl`.
//!
//! The id is `session_meta.payload.id`. A file already in the pre-launch
//! snapshot is never chosen. Two new files are correlated by the first user
//! item, and a remaining tie is [`ThreadIdError::Ambiguous`].
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const TIMESTAMP_SKEW: Duration = Duration::from_secs(2);

use crewmux_transcript::{
    CodexRollout, Error as TranscriptError, TranscriptCursor, TranscriptSource, read_session_meta,
};
use crewmux_types::{RuntimeKind, ThreadId};

/// Why a rollout id could not be chosen.
#[derive(Debug, thiserror::Error)]
pub enum ThreadIdError {
    /// No eligible file yet, or the first user item has not been written.
    #[error("thread id is not ready")]
    NotYet,
    /// More than one file still matches.
    #[error("ambiguous rollouts ({})", .0.len())]
    Ambiguous(Vec<PathBuf>),
    /// A candidate file could not be read as rollout metadata.
    #[error("malformed rollout {}", .0.display())]
    Malformed(PathBuf),
}

/// Paths of rollout files observed before exec.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RolloutSet {
    paths: BTreeSet<PathBuf>,
}

impl RolloutSet {
    /// Whether `path` was present before launch.
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        self.paths.contains(path)
    }

    /// How many files were snapshotted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// No files were present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// `$CODEX_HOME/sessions`.
#[derive(Clone, Debug)]
pub struct RolloutLocator {
    /// Sessions directory.
    pub sessions_root: PathBuf,
}

impl RolloutLocator {
    /// Byte length of each rollout file. A file created later is absent.
    #[must_use]
    pub fn byte_baseline(&self) -> BTreeMap<PathBuf, u64> {
        let mut lengths = BTreeMap::new();
        for path in list_rollouts(&self.sessions_root) {
            let path = canonical(&path);
            let len = fs::metadata(&path).map_or(0, |meta| meta.len());
            lengths.insert(path, len);
        }
        lengths
    }

    /// Every rollout file currently on disk.
    #[must_use]
    pub fn snapshot(&self) -> RolloutSet {
        let mut paths = BTreeSet::new();
        for path in list_rollouts(&self.sessions_root) {
            paths.insert(canonical(&path));
        }
        RolloutSet { paths }
    }

    /// Pick the rollout created for this launch.
    ///
    /// `submitted` is the first user body, used only when more than one new
    /// file matches the cwd. `None` leaves that case as [`ThreadIdError::NotYet`].
    ///
    /// # Errors
    ///
    /// Returns [`ThreadIdError`] when the id is missing, tied, or the file is malformed.
    pub fn resolve(
        &self,
        before: &RolloutSet,
        cwd: &Path,
        launched_at: SystemTime,
        submitted: Option<&str>,
    ) -> Result<ThreadId, ThreadIdError> {
        let mut found = Vec::new();
        for path in list_rollouts(&self.sessions_root) {
            let path = canonical(&path);
            if before.contains(&path) {
                continue;
            }
            let file = match read_rollout(&path) {
                Ok(file) => file,
                Err(ThreadIdError::NotYet) => continue,
                Err(err) => return Err(err),
            };
            let floor = launched_at
                .checked_sub(TIMESTAMP_SKEW)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if file.at < floor || !cwd_eq(cwd, &file.cwd) {
                continue;
            }
            found.push(file);
        }
        found.sort_by(|left, right| right.at.cmp(&left.at).then(left.path.cmp(&right.path)));
        pick(found, submitted)
    }

    /// The rollout for `id` still exists and its cwd matches.
    ///
    /// # Errors
    ///
    /// Returns [`ThreadIdError::NotYet`] when the file is gone, [`ThreadIdError::Malformed`]
    /// when the cwd differs, and [`ThreadIdError::Ambiguous`] when two files share the id.
    pub fn verify_resume(&self, id: &ThreadId, cwd: &Path) -> Result<PathBuf, ThreadIdError> {
        let mut matches = Vec::new();
        for path in list_rollouts(&self.sessions_root) {
            let path = canonical(&path);
            let file = match read_rollout(&path) {
                Ok(file) => file,
                Err(ThreadIdError::NotYet) => continue,
                Err(err) => return Err(err),
            };
            if file.id.as_str() == id.as_str() {
                matches.push(file);
            }
        }
        let file = one_file(matches)?;
        if cwd_eq(cwd, &file.cwd) {
            Ok(file.path)
        } else {
            Err(ThreadIdError::Malformed(file.path))
        }
    }
}

/// User-role texts in rollout order.
///
/// # Errors
///
/// Returns [`ThreadIdError::Malformed`] when a complete line is not UTF-8 JSON.
/// A partial last line, including one cut mid-character, is skipped.
pub fn user_turn_texts(path: &Path) -> Result<Vec<String>, ThreadIdError> {
    user_turns_after(path, 0)
}

/// User texts whose jsonl line starts at or after `baseline`.
///
/// # Errors
///
/// Same as [`user_turn_texts`]. A file with no complete line is [`ThreadIdError::NotYet`].
pub fn user_turns_after(path: &Path, baseline: u64) -> Result<Vec<String>, ThreadIdError> {
    let rollout = CodexRollout::open(path.to_path_buf());
    match rollout.user_turns_after(TranscriptCursor(baseline)) {
        Ok(turns) => {
            if turns.is_empty() && incomplete_only(path) {
                return Err(ThreadIdError::NotYet);
            }
            Ok(turns.into_iter().map(|turn| turn.text).collect())
        }
        Err(err) => Err(map_transcript(path, &err)),
    }
}

fn incomplete_only(path: &Path) -> bool {
    matches!(
        read_session_meta(path),
        Err(TranscriptError::Incomplete { .. })
    )
}

struct RolloutFile {
    path: PathBuf,
    id: ThreadId,
    cwd: PathBuf,
    at: SystemTime,
    users: Vec<String>,
}

fn pick(found: Vec<RolloutFile>, submitted: Option<&str>) -> Result<ThreadId, ThreadIdError> {
    match found.len() {
        0 => Err(ThreadIdError::NotYet),
        1 => {
            let file = found.into_iter().next().ok_or(ThreadIdError::NotYet)?;
            Ok(file.id)
        }
        _ => correlate(found, submitted),
    }
}

fn correlate(found: Vec<RolloutFile>, submitted: Option<&str>) -> Result<ThreadId, ThreadIdError> {
    let Some(body) = submitted else {
        return Err(ThreadIdError::NotYet);
    };
    let mut hits = Vec::new();
    for file in found {
        // `users` is the same filter as delivery: response_item role=user
        // input_text, without environment_context, AGENTS.md, or event_msg.
        let Some(first) = file.users.first() else {
            continue;
        };
        if crewmux_transcript::match_turn(RuntimeKind::Codex, body, first)
            == crewmux_transcript::TurnMatch::Equal
        {
            hits.push(file);
        }
    }
    match hits.len() {
        0 => Err(ThreadIdError::NotYet),
        1 => {
            let file = hits.into_iter().next().ok_or(ThreadIdError::NotYet)?;
            Ok(file.id)
        }
        _ => Err(ThreadIdError::Ambiguous(
            hits.into_iter().map(|file| file.path).collect(),
        )),
    }
}

fn one_file(matches: Vec<RolloutFile>) -> Result<RolloutFile, ThreadIdError> {
    match matches.len() {
        0 => Err(ThreadIdError::NotYet),
        1 => matches.into_iter().next().ok_or(ThreadIdError::NotYet),
        _ => Err(ThreadIdError::Ambiguous(
            matches.into_iter().map(|file| file.path).collect(),
        )),
    }
}

fn read_rollout(path: &Path) -> Result<RolloutFile, ThreadIdError> {
    let meta = read_session_meta(path).map_err(|err| map_transcript(path, &err))?;
    let id = ThreadId::new(&meta.id).map_err(|_| ThreadIdError::Malformed(path.to_path_buf()))?;
    let users = user_turn_texts(path)?;
    Ok(RolloutFile {
        path: path.to_path_buf(),
        id,
        cwd: meta.cwd,
        at: meta.timestamp,
        users,
    })
}

fn map_transcript(path: &Path, err: &TranscriptError) -> ThreadIdError {
    match err {
        TranscriptError::Incomplete { .. } => ThreadIdError::NotYet,
        TranscriptError::Io(_) | TranscriptError::Parse { .. } | TranscriptError::SessionId => {
            ThreadIdError::Malformed(path.to_path_buf())
        }
    }
}

fn list_rollouts(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if is_rollout(&path) {
            out.push(path);
        }
    }
}

fn is_rollout(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("rollout-")
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn cwd_eq(seat: &Path, recorded: &Path) -> bool {
    let seat_c = fs::canonicalize(seat).unwrap_or_else(|_| seat.to_path_buf());
    let recorded_c = fs::canonicalize(recorded).unwrap_or_else(|_| recorded.to_path_buf());
    seat_c == recorded_c
}
