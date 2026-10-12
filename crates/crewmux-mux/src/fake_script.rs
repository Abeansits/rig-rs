//! In-memory stand-in for `rig-stub-agent`. The grammar matches `crewmux-adapters`.

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
