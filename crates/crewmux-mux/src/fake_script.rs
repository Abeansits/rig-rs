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
