// Derived from openrig packages/tui/src/state.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Group a flat seat list into rigs and pods, and the lines each section paints.

use crewmux_api::{PodName, ReadinessCode, RigName, SeatKey, SeatState, SeatView, Snapshot};

use crate::seat_bits::{RuntimeBits, SeatBits};

use crate::action::{Action, CrossLink};
use crate::state::{RowKey, Section, State};

/// One content-pane line and the hit action painted with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContentRow {
    /// Visible text.
    pub(crate) text: String,
    /// Hit action for this line, when the line is clickable.
    pub(crate) action: Option<Action>,
}

/// One visible explorer row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisibleRow {
    /// Identity.
    pub(crate) key: RowKey,
    /// Indent depth. `0` is a rig, `1` a pod, `2` a seat.
    pub(crate) depth: u8,
    /// This row has a disclosure glyph.
    pub(crate) disclosure: bool,
    /// The glyph is the open one.
    pub(crate) open: bool,
    /// Text after the indent.
    pub(crate) title: String,
}

/// A rig and the pods first seen under it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RigNode {
    id: RigName,
    name: RigName,
    pods: Vec<PodNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PodNode {
    name: PodName,
    seats: Vec<SeatView>,
}

/// Explorer rows the operator can see, in paint order.
#[must_use]
pub(crate) fn visible_rows(state: &State) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for rig in group(&state.snapshot) {
        let rig_key = RowKey::Rig(rig.id.clone());
        let open = state.expanded.contains(&rig_key);
        rows.push(VisibleRow {
            key: rig_key.clone(),
            depth: 0,
            disclosure: true,
            open,
            title: rig.name.as_str().to_owned(),
        });
        if !open {
            continue;
        }
        push_pods(state, &rig, &mut rows);
    }
    rows
}

fn push_pods(state: &State, rig: &RigNode, rows: &mut Vec<VisibleRow>) {
    for pod in &rig.pods {
        let key = RowKey::Pod {
            rig: rig.id.clone(),
            pod: pod.name.clone(),
        };
        let open = state.expanded.contains(&key);
        rows.push(VisibleRow {
            key,
            depth: 1,
            disclosure: true,
            open,
            title: pod.name.as_str().to_owned(),
        });
        if open {
            push_seats(pod, rows);
        }
    }
}

fn push_seats(pod: &PodNode, rows: &mut Vec<VisibleRow>) {
    for seat in &pod.seats {
        rows.push(VisibleRow {
            key: RowKey::Seat(seat.id.clone()),
            depth: 2,
            disclosure: false,
            open: false,
            title: format!("{}  {}", seat.name.as_str(), seat.state.label()),
        });
    }
}

/// Disclosure keys in the snapshot, rigs then pods.
#[must_use]
pub(crate) fn disclosure_keys(snapshot: &Snapshot) -> Vec<RowKey> {
    let mut keys = Vec::new();
    for rig in group(snapshot) {
        keys.push(RowKey::Rig(rig.id.clone()));
        for pod in rig.pods {
            keys.push(RowKey::Pod {
                rig: rig.id.clone(),
                pod: pod.name,
            });
        }
    }
    keys
}

/// Find a seat by id.
#[must_use]
pub(crate) fn find_seat<'a>(snapshot: &'a Snapshot, id: &SeatKey) -> Option<&'a SeatView> {
    snapshot.seats.iter().find(|seat| &seat.id == id)
}

/// Lines of the content pane, before the scroll window.
#[must_use]
pub(crate) fn content_rows(state: &State) -> Vec<ContentRow> {
    match state.section {
        Section::Explorer => explorer_copy(state),
        Section::Detail => detail_lines(state),
        Section::Attention => attention_lines(state),
        Section::Health => health_lines(state),
        Section::Activity => activity_lines(state),
    }
}

fn plain(text: impl Into<String>) -> ContentRow {
    ContentRow {
        text: text.into(),
        action: None,
    }
}

fn explorer_copy(state: &State) -> Vec<ContentRow> {
    let Some(key) = state.selection.clone() else {
        return if state.snapshot.seats.is_empty() {
            vec![plain("no seats")]
        } else {
            vec![plain("select a row")]
        };
    };
    match key {
        RowKey::Seat(id) => detail_for(state, &id),
        RowKey::Rig(id) => vec![plain(format!("rig {id}"))],
        RowKey::Pod { rig, pod } => vec![plain(format!("pod {pod} in {rig}"))],
    }
}

fn detail_lines(state: &State) -> Vec<ContentRow> {
    let Some(RowKey::Seat(id)) = state.selection.clone() else {
        return vec![plain("no seat selected")];
    };
    detail_for(state, &id)
}

fn detail_for(state: &State, id: &SeatKey) -> Vec<ContentRow> {
    let Some(seat) = find_seat(&state.snapshot, id) else {
        return vec![plain("seat is gone")];
    };
    let mut lines = vec![
        plain(format!("{}  {}", seat.name.as_str(), seat.state.label())),
        plain(format!("runtime {}", seat.runtime.label())),
    ];
    if let Some(session) = &seat.session {
        lines.push(plain(format!("session {session}")));
    }
    if let Some(code) = seat.state.attention_code() {
        lines.push(plain(format!("reason {}", code_name(code))));
    }
    if let Some(spec) = &seat.spec {
        lines.push(ContentRow {
            text: format!("spec {spec}"),
            action: Some(Action::Cross(CrossLink::Spec { name: spec.clone() })),
        });
    }
    if seat.state.can_attach() && seat.session.is_some() {
        lines.push(ContentRow {
            text: "attach".to_owned(),
            action: Some(Action::Attach(seat.id.clone())),
        });
    }
    lines
}

fn attention_lines(state: &State) -> Vec<ContentRow> {
    let mut lines = Vec::new();
    for seat in &state.snapshot.seats {
        if let SeatState::Attention(code) = &seat.state {
            lines.push(ContentRow {
                text: format!("{}  {}", seat.name.as_str(), code_name(code)),
                action: Some(Action::Select(RowKey::Seat(seat.id.clone()))),
            });
        }
    }
    if lines.is_empty() {
        vec![plain("no attention")]
    } else {
        lines
    }
}

fn health_lines(state: &State) -> Vec<ContentRow> {
    match &state.snapshot.health {
        None => vec![plain("health: unavailable")],
        Some(summary) if summary.records.is_empty() => vec![plain("health: no findings")],
        Some(summary) => summary
            .records
            .iter()
            .map(|item| plain(format!("{}  {}", item.severity, item.summary)))
            .collect(),
    }
}

fn activity_lines(state: &State) -> Vec<ContentRow> {
    if state.snapshot.activity.is_empty() {
        return vec![plain("no activity")];
    }
    state
        .snapshot
        .activity
        .iter()
        .map(|item| plain(format!("{}  {}", item.seat.as_str(), item.summary)))
        .collect()
}

/// Wire name of a readiness code, matching the serde rename.
#[must_use]
pub(crate) fn code_name(code: &ReadinessCode) -> String {
    match code {
        ReadinessCode::Ready => "ready".to_owned(),
        ReadinessCode::Booting => "booting".to_owned(),
        ReadinessCode::TrustGate => "trust_gate".to_owned(),
        ReadinessCode::HookTrustGate => "hook_trust_gate".to_owned(),
        ReadinessCode::UpdateGate => "update_gate".to_owned(),
        ReadinessCode::LoginRequired => "login_required".to_owned(),
        ReadinessCode::McpGate => "mcp_gate".to_owned(),
        ReadinessCode::BypassConsentGate => "bypass_consent_gate".to_owned(),
        ReadinessCode::CodexAuthRefusal => "codex_auth_refusal".to_owned(),
        ReadinessCode::CodexClientIncompatible => "codex_client_incompatible".to_owned(),
        ReadinessCode::OnboardingTheme => "onboarding_theme".to_owned(),
        ReadinessCode::UnknownMenu => "unknown_menu".to_owned(),
        ReadinessCode::UnrecognisedScreen => "unrecognised_screen".to_owned(),
        ReadinessCode::Other(other) => other.clone(),
    }
}

fn group(snapshot: &Snapshot) -> Vec<RigNode> {
    let mut rigs: Vec<RigNode> = Vec::new();
    for seat in &snapshot.seats {
        let rig_index = rig_index(&mut rigs, seat);
        let Some(rig) = rigs.get_mut(rig_index) else {
            continue;
        };
        let pod_index = pod_index(rig, seat);
        let Some(pod) = rig.pods.get_mut(pod_index) else {
            continue;
        };
        pod.seats.push(seat.clone());
    }
    rigs
}

fn rig_index(rigs: &mut Vec<RigNode>, seat: &SeatView) -> usize {
    if let Some(index) = rigs.iter().position(|rig| rig.id == seat.rig_id) {
        return index;
    }
    rigs.push(RigNode {
        id: seat.rig_id.clone(),
        name: seat.rig_name.clone(),
        pods: Vec::new(),
    });
    rigs.len().saturating_sub(1)
}

fn pod_index(rig: &mut RigNode, seat: &SeatView) -> usize {
    if let Some(index) = rig.pods.iter().position(|pod| pod.name == seat.pod) {
        return index;
    }
    rig.pods.push(PodNode {
        name: seat.pod.clone(),
        seats: Vec::new(),
    });
    rig.pods.len().saturating_sub(1)
}
