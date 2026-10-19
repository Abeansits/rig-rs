// Derived from openrig packages/daemon/src/domain/watchdog-policy-engine.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! The small YAML slice 14 stores on an armed keepalive.

/// Fields the outbox loop reads off an armed spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmedSpec {
    /// Policy name. Empty when the line is absent.
    pub policy: String,
    /// `target.session`.
    pub target_session: String,
    /// `context.workflow_instance_id`.
    pub instance_id: String,
    /// Top-level or context `deadline_gated`.
    pub deadline_gated: bool,
    /// Optional observer.
    pub observer_session: String,
    /// Optional authored message. One line.
    pub message: String,
}

/// Parse the armed keepalive YAML. Unknown lines are ignored.
#[must_use]
pub fn parse_armed(yaml: &str) -> ArmedSpec {
    let mut spec = ArmedSpec {
        policy: String::new(),
        target_session: String::new(),
        instance_id: String::new(),
        deadline_gated: false,
        observer_session: String::new(),
        message: String::new(),
    };
    for line in yaml.lines() {
        let line = line.trim_end();
        if let Some(value) = value_of(line, "policy:") {
            spec.policy = value;
        } else if let Some(value) = value_of(line, "session:") {
            spec.target_session = value;
        } else if let Some(value) = value_of(line, "workflow_instance_id:") {
            spec.instance_id = value;
        } else if let Some(value) = value_of(line, "deadline_gated:") {
            spec.deadline_gated = value == "true";
        } else if let Some(value) = value_of(line, "observer_session:") {
            spec.observer_session = value;
        } else if let Some(value) = value_of(line, "message:") {
            spec.message = value;
        }
    }
    spec
}

fn value_of(line: &str, key: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix(key)?.trim();
    if rest.is_empty() {
        return None;
    }
    Some(rest.to_owned())
}
