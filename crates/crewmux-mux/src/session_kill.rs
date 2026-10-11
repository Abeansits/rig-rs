//! Drop a seat session without leaving its window in `crewmux-control`.
//!
//! `link-window` shares the seat window with the control session so `%output`
//! reaches the control client. `kill-session` does not destroy a window that
//! is still linked elsewhere, so the pane process keeps running. Unlink that
//! window from the control session first. The bell window is only in the
//! control session and is left alone.

use std::collections::BTreeSet;

use crate::argv::{canonical_session_name, kill_session_args};
use crate::error::MuxError;
use crate::exec::TmuxExec;
use crate::types::CONTROL_SESSION;

fn list_window_links_args() -> Vec<String> {
    vec![
        "list-windows".to_owned(),
        "-a".to_owned(),
        "-F".to_owned(),
        "#{window_id}|#{session_name}".to_owned(),
    ]
}

fn unlink_window_args(target: &str) -> Vec<String> {
    vec![
        "unlink-window".to_owned(),
        "-t".to_owned(),
        target.to_owned(),
    ]
}

/// Unlink the seat's windows from the control session, then kill the session.
pub(crate) async fn unlink_and_kill(exec: &dyn TmuxExec, name: &str) -> Result<(), MuxError> {
    let session = canonical_session_name(name);
    let listed = exec.run(&list_window_links_args()).await?;
    for window in windows_linked_into_control(&listed.stdout, &session) {
        let target = format!("{CONTROL_SESSION}:{window}");
        exec.run(&unlink_window_args(&target)).await?;
    }
    exec.run(&kill_session_args(name)).await.map(|_| ())
}

/// Window ids present in both `session` and the control session.
pub(crate) fn windows_linked_into_control(stdout: &str, session: &str) -> Vec<String> {
    let mut seat = BTreeSet::new();
    let mut control = BTreeSet::new();
    for line in stdout.lines() {
        let Some((id, owner)) = line.split_once('|') else {
            continue;
        };
        if !is_window_id(id) || owner.is_empty() {
            continue;
        }
        if owner == session {
            seat.insert(id);
        } else if owner == CONTROL_SESSION {
            control.insert(id);
        }
    }
    seat.intersection(&control)
        .map(|id| (*id).to_owned())
        .collect()
}

fn is_window_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix('@') else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::windows_linked_into_control;

    #[test]
    fn only_windows_linked_into_control_are_unlinked() {
        let text = "\
@0|crewmux-control
@1|crewmux-control
@1|seat
@2|seat
@3|other
";
        assert_eq!(
            windows_linked_into_control(text, "seat"),
            vec!["@1".to_owned()]
        );
    }
}
