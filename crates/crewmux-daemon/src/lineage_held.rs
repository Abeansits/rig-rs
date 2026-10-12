//! Open files of a live pane process.
//!
//! `exec` keeps the pid and the start time, and a version title can erase the
//! session id from the command line. The id has to appear in an open transcript
//! or session file, or the send is refused.

use std::path::Path;

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
    use super::process_holds_session;

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
