//! In-memory stand-in for `rig-stub-agent`. The grammar matches `crewmux-adapters`.

use crate::types::{Capture, CaptureRange};

/// Parsed `RIG_STUB_SCRIPT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StubPlan {
    pub(crate) ready_after_ms: u64,
    pub(crate) exit_after_ms: Option<u64>,
    pub(crate) gate: Option<String>,
}

pub(crate) fn parse_stub(raw: &str) -> StubPlan {
    let mut plan = StubPlan {
        ready_after_ms: 0,
        exit_after_ms: None,
        gate: None,
    };
    for part in raw.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        match key.trim() {
            "ready_after_ms" => plan.ready_after_ms = value.trim().parse().unwrap_or(0),
            "exit_after_ms" => plan.exit_after_ms = value.trim().parse().ok(),
            "print_gate" => plan.gate = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    plan
}

pub(crate) fn render_stub(plan: &StubPlan) -> String {
    let mut text = String::from("stub-agent\n");
    if let Some(code) = &plan.gate {
        text.push_str("stub gate: ");
        text.push_str(code);
        text.push('\n');
    } else {
        text.push_str("stub> \n");
    }
    text
}

pub(crate) fn banner() -> &'static str {
    "stub-agent\n"
}

/// Stub script wins. Otherwise a debug `RIG_FAKE_CAPTURE` can show a Claude pane.
pub(crate) fn session_screen(plan: &StubPlan, stub_script: Option<&str>) -> String {
    if stub_script.is_none()
        && let Some(text) = debug_capture_override()
    {
        return text;
    }
    render_stub(plan)
}

/// Append `text` to `RIG_PASTE_SINK` when that path is set.
///
/// Debug builds only. The product confirm path still reads the Claude jsonl.
/// A test copies the echoed paste into that file; it does not skip the check.
/// Callers must not run this on a tokio worker thread.
pub(crate) fn echo_paste(text: &str) {
    #[cfg(debug_assertions)]
    {
        if std::env::var_os("RIG_PASTE_SINK").is_none() {
            return;
        }
        let owned = text.to_owned();
        drop(tokio::task::spawn_blocking(move || note_paste(&owned)));
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = text;
    }
}

fn note_paste(text: &str) {
    let Ok(path) = std::env::var("RIG_PASTE_SINK") else {
        return;
    };
    let mut record = Vec::new();
    record.extend(text.len().to_string().into_bytes());
    record.push(b'\n');
    record.extend(text.as_bytes());
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    drop(std::io::Write::write_all(&mut file, &record));
}

/// After Enter, rewrite a stub screen so the prompt is showing again.
///
/// A Claude composer has no `stub>` marker, so it stays as it is.
pub(crate) fn restore_stub_prompt(text: &str) -> Option<String> {
    if !text.contains("stub>") {
        return None;
    }
    if text.ends_with("stub> \n") {
        return Some(text.to_owned());
    }
    let mut next = text.to_owned();
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str("stub> \n");
    Some(next)
}

/// Insert `pasted` after the last `›`, keeping a closing box bar on the last line.
pub(crate) fn spliced_composer(capture: &str, pasted: &str) -> String {
    let Some(cursor) = capture.rfind('›') else {
        return capture.to_owned();
    };
    let Some(mark) = capture.get(cursor..).and_then(|rest| rest.chars().next()) else {
        return capture.to_owned();
    };
    let prefix_end = cursor.saturating_add(mark.len_utf8());
    let line_start = capture
        .get(..cursor)
        .and_then(|head| head.rfind('\n'))
        .map_or(0, |index| index.saturating_add(1));
    let line_end = capture
        .get(cursor..)
        .and_then(|rest| rest.find('\n'))
        .map_or(capture.len(), |index| cursor.saturating_add(index));
    let Some(prefix) = capture.get(line_start..prefix_end) else {
        return capture.to_owned();
    };
    let trailing = capture
        .get(prefix_end..line_end)
        .is_some_and(|tail| tail.trim_end().ends_with(['│', '┃']));
    let suffix = if trailing { " │" } else { "" };
    let mut rebuilt = String::new();
    rebuilt.push_str(capture.get(..line_start).unwrap_or(""));
    rebuilt.push_str(prefix);
    rebuilt.push(' ');
    rebuilt.push_str(pasted);
    rebuilt.push_str(suffix);
    rebuilt.push_str(capture.get(line_end..).unwrap_or(""));
    rebuilt
}

/// Keep the requested tail of a capture, including a trailing newline.
pub(crate) fn slice_capture(capture: Capture, range: CaptureRange) -> Capture {
    let lines = match range {
        CaptureRange::Visible | CaptureRange::Joined => return capture,
        CaptureRange::LastLines(n) => usize::from(n),
        CaptureRange::History(n) | CaptureRange::JoinedHistory(n) => {
            usize::try_from(n).unwrap_or(usize::MAX)
        }
    };
    let trailing = capture.text.ends_with('\n');
    let kept: Vec<&str> = capture.text.lines().collect();
    let count = kept.len();
    let skip = count.saturating_sub(lines);
    let mut text = kept.into_iter().skip(skip).collect::<Vec<_>>().join("\n");
    // `lines` drops the terminator. An exact append is `before + payload`, so a
    // prompt that ended with a newline has to keep it or every paste looks partial.
    if trailing && skip < count {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}

fn debug_capture_override() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        let raw = std::env::var("RIG_FAKE_CAPTURE").ok()?;
        if raw == "claude-ready" {
            let rule = "─".repeat(40);
            return Some(format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"));
        }
        Some(raw)
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}
