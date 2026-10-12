//! Session proof when the command line has no session token.
//!
//! `exec` keeps the pid and the start time, and a version title can erase the
//! session id from the command line. Claude Code 2.1.286 writes
//! `~/.claude/projects/<cwd>/<sid>.jsonl` and closes it; the fd does not stay
//! open. Proof is that open fd, or that file being the newest transcript in
//! the project directory.

use std::path::Path;

/// `true` when an open fd names `session_id`, or the newest Claude transcript
/// for `cwd` is that session.
///
/// `cwd` is the seat directory. An empty path falls back to `/proc/<pid>/cwd`.
/// A missing project directory is not proof.
pub(crate) async fn session_is_proven(pid: u32, session_id: &str, cwd: Option<&Path>) -> bool {
    if session_id.is_empty() {
        return false;
    }
    if process_holds_session(pid, session_id).await {
        return true;
    }
    let session = session_id.to_owned();
    let cwd = cwd.map(Path::to_path_buf);
    tokio::task::spawn_blocking(move || transcript_on_disk(pid, &session, cwd.as_deref()))
        .await
        .unwrap_or(false)
}

/// Claude's project-directory name: every non-alphanumeric byte becomes `-`.
///
/// `/tmp/claude-ev-print/work` is `-tmp-claude-ev-print-work` on Claude Code 2.1.286.
#[must_use]
pub(crate) fn encode_project(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn transcript_on_disk(pid: u32, session_id: &str, cwd: Option<&Path>) -> bool {
    let cwd = match cwd {
        Some(path) if !path.as_os_str().is_empty() => path.to_path_buf(),
        _ => match std::fs::read_link(format!("/proc/{pid}/cwd")) {
            Ok(path) => path,
            Err(_) => return false,
        },
    };
    let Some(config) = claude_config_dir(pid) else {
        return false;
    };
    let dir = config.join("projects").join(encode_project(&cwd));
    newest_is_session(&dir, session_id)
}

fn claude_config_dir(pid: u32) -> Option<std::path::PathBuf> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    if let Some(dir) = env_value(&raw, "CLAUDE_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    let home = env_value(&raw, "HOME")?;
    Some(std::path::PathBuf::from(home).join(".claude"))
}

fn env_value(raw: &[u8], key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    for entry in raw.split(|byte| *byte == 0) {
        let Ok(entry) = std::str::from_utf8(entry) else {
            continue;
        };
        if let Some(value) = entry.strip_prefix(&prefix)
            && !value.is_empty()
        {
            return Some(value.to_owned());
        }
    }
    None
}

/// `true` when `<session_id>.jsonl` exists and is strictly newer than every
/// other `*.jsonl` in `dir`.
fn newest_is_session(dir: &Path, session_id: &str) -> bool {
    let expected = format!("{session_id}.jsonl");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut session_mtime = None;
    let mut newest_other = None;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !std::path::Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
        {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if name == expected {
            session_mtime = Some(modified);
        } else {
            newest_other = Some(match newest_other {
                Some(prev) if prev > modified => prev,
                _ => modified,
            });
        }
    }
    match (session_mtime, newest_other) {
        (Some(session), Some(other)) => session > other,
        (Some(_), None) => true,
        _ => false,
    }
}

/// `true` when `pid` holds an open file whose path contains `session_id`.
///
/// Linux reads `/proc/<pid>/fd`. Other platforms ask `lsof`. A missing
/// directory or a failed `lsof` is not proof.
pub(crate) async fn process_holds_session(pid: u32, session_id: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    let session = session_id.to_owned();
    let from_proc = tokio::task::spawn_blocking(move || proc_fds(pid, &session))
        .await
        .ok()
        .flatten();
    if let Some(found) = from_proc {
        return found;
    }
    lsof_holds(pid, session_id).await
}

/// `Some` when `/proc/<pid>/fd` can be read. `None` when this is not Linux
/// or the process is not visible.
fn proc_fds(pid: u32, session_id: &str) -> Option<bool> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut found = false;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if path_names_session(&target, session_id) {
            found = true;
            break;
        }
    }
    Some(found)
}

fn path_names_session(path: &Path, session_id: &str) -> bool {
    path.to_string_lossy().contains(session_id)
}

async fn lsof_holds(pid: u32, session_id: &str) -> bool {
    let output = tokio::process::Command::new("lsof")
        .args(["-n", "-P", "-p", &pid.to_string(), "-Fn"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return false;
    };
    text.lines().any(|line| {
        line.strip_prefix('n')
            .is_some_and(|path| path.contains(session_id))
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{encode_project, newest_is_session, process_holds_session};

    #[test]
    fn project_dir_matches_claude_code_2_1_286() {
        let cwd = std::path::Path::new("/tmp/claude-ev-print/work");
        assert_eq!(encode_project(cwd), "-tmp-claude-ev-print-work");
    }

    #[test]
    fn newest_transcript_proves_the_session_and_a_later_one_does_not() {
        let dir = std::env::temp_dir().join(format!("rig-newest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let session = "11111111-1111-4111-8111-111111111111";
        let other = "22222222-2222-4222-8222-222222222222";
        write_at(&dir.join(format!("{session}.jsonl")), 10);
        assert!(newest_is_session(&dir, session));
        write_at(&dir.join(format!("{other}.jsonl")), 20);
        assert!(!newest_is_session(&dir, session));
        assert!(newest_is_session(&dir, other));
        drop(std::fs::remove_dir_all(&dir));
    }

    fn hex_sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let digest = Sha256::digest(bytes);
        let mut out = String::with_capacity(digest.len().saturating_mul(2));
        for byte in digest {
            let high = usize::from(byte >> 4);
            let low = usize::from(byte & 0x0f);
            if let (Some(high), Some(low)) = (HEX.get(high), HEX.get(low)) {
                out.push(char::from(*high));
                out.push(char::from(*low));
            }
        }
        out
    }

    fn write_at(path: &std::path::Path, secs: u64) {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .expect("transcript");
        file.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
            .expect("mtime");
    }

    #[test]
    fn claude_code_fd_fixture_matches_sha256() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude-code-2.1.286-fds");
        let labels = std::fs::read_to_string(root.join("sha256.jsonl")).expect("labels");
        let mut seen = 0usize;
        for line in labels.lines() {
            let row: serde_json::Value = serde_json::from_str(line).expect("row");
            let file = row
                .get("file")
                .and_then(serde_json::Value::as_str)
                .expect("file");
            let expect = row
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .expect("sha");
            let bytes = std::fs::read(root.join(file)).expect("fixture");
            let digest = hex_sha256(&bytes);
            assert_eq!(digest, expect, "{file}");
            seen = seen.saturating_add(1);
        }
        assert!(seen >= 8, "{seen}");
    }

    #[tokio::test]
    async fn an_open_file_names_the_session_and_a_different_id_does_not() {
        let dir = std::env::temp_dir().join(format!("rig-held-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let session = "11111111-1111-4111-8111-111111111111";
        let path = dir.join(format!("{session}.jsonl"));
        let held = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("hold");
        assert!(process_holds_session(std::process::id(), session).await);
        assert!(
            !process_holds_session(std::process::id(), "22222222-2222-4222-8222-222222222222")
                .await
        );
        drop(held);
        drop(std::fs::remove_dir_all(&dir));
    }
}
