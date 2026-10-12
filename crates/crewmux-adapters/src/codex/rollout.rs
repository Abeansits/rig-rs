//! Thread ids from `$CODEX_HOME/sessions/**/rollout-*.jsonl`.
//!
//! The id is `session_meta.payload.id`. A file already in the pre-launch
//! snapshot is never chosen. Two new files are correlated by the first user
//! item, and a remaining tie is [`ThreadIdError::Ambiguous`].

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Rollout timestamps are whole seconds. A file written in the same second as
/// launch can look older once the fraction is dropped.
const TIMESTAMP_SKEW: Duration = Duration::from_secs(2);

use serde_json::Value;

use crewmux_types::{RuntimeKind, ThreadId};

use crate::codex::normalize::same_delivery;
use crate::codex::timeutil::parse_rfc3339;

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
            let file = read_rollout(&path)?;
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
            let file = read_rollout(&path)?;
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
/// Returns [`ThreadIdError::Malformed`] when a line is not JSON.
pub fn user_turn_texts(path: &Path) -> Result<Vec<String>, ThreadIdError> {
    Ok(read_rollout(path)?.users)
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
        let Some(first) = file.users.first() else {
            continue;
        };
        if same_delivery(RuntimeKind::Codex, body, first) {
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
    let text =
        fs::read_to_string(path).map_err(|_| ThreadIdError::Malformed(path.to_path_buf()))?;
    let mut meta = None;
    let mut users = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(line).map_err(|_| ThreadIdError::Malformed(path.to_path_buf()))?;
        if meta.is_none() {
            meta = session_meta(&value, path)?;
        }
        if let Some(turn) = user_text(&value) {
            users.push(turn);
        }
    }
    let Some((id, cwd, at)) = meta else {
        return Err(ThreadIdError::Malformed(path.to_path_buf()));
    };
    Ok(RolloutFile {
        path: path.to_path_buf(),
        id,
        cwd,
        at,
        users,
    })
}

fn session_meta(
    value: &Value,
    path: &Path,
) -> Result<Option<(ThreadId, PathBuf, SystemTime)>, ThreadIdError> {
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let payload = value
        .get("payload")
        .ok_or_else(|| ThreadIdError::Malformed(path.to_path_buf()))?;
    let id_text = payload
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ThreadIdError::Malformed(path.to_path_buf()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if !name.contains(id_text) {
        return Err(ThreadIdError::Malformed(path.to_path_buf()));
    }
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| ThreadIdError::Malformed(path.to_path_buf()))?;
    let stamp = payload
        .get("timestamp")
        .and_then(Value::as_str)
        .or_else(|| value.get("timestamp").and_then(Value::as_str))
        .ok_or_else(|| ThreadIdError::Malformed(path.to_path_buf()))?;
    let at = parse_rfc3339(stamp).ok_or_else(|| ThreadIdError::Malformed(path.to_path_buf()))?;
    let id = ThreadId::new(id_text).map_err(|_| ThreadIdError::Malformed(path.to_path_buf()))?;
    Ok(Some((id, PathBuf::from(cwd), at)))
}

fn user_text(value: &Value) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    let role = payload.get("role").and_then(Value::as_str);
    let kind = payload.get("type").and_then(Value::as_str);
    if role != Some("user") && kind != Some("user_message") {
        return None;
    }
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(message) = payload.get("message").and_then(Value::as_str) {
        return Some(message.to_owned());
    }
    let content = payload.get("content")?.as_array()?;
    let mut combined = String::new();
    for item in content {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            combined.push_str(text);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
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
