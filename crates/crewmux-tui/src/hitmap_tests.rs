//! Recorded `OpenRig` v0.6.9 hit maps, compared by action kind and target.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crewmux_api::{
    PodName, RigId, RigName, RuntimeKind, SeatId, SeatName, SeatState, SeatView, Snapshot,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::Value;

use crate::action::{Action, CrossLink, HitMap};
use crate::state::{RowKey, Section, Size, State};
use crate::update::update;
use crate::view::view;

#[test]
fn recorded_openrig_screens_match_action_identity() {
    let explorer = load("explorer-expanded-pod.json");
    let detail = load("seat-detail-spec-cross.json");
    let attention = load("attention-list.json");
    let explorer_ids = corpus_idents(&explorer, &detail, &attention);
    assert_rust_matches(&explorer_ids);
}

fn corpus_idents(explorer: &Value, detail: &Value, attention: &Value) -> BTreeSet<String> {
    for screen in [explorer, detail, attention] {
        assert_eq!(screen["source"].as_str(), Some("openrig v0.6.9"));
        let reason = screen["shift_reason"].as_str().unwrap_or("");
        assert!(
            reason.contains("ADR-0004"),
            "shift reason must name ADR-0004: {reason}"
        );
        assert!(
            screen["hits"]
                .as_array()
                .is_some_and(|hits| !hits.is_empty())
        );
    }
    let explorer_ids = idents(explorer);
    let detail_ids = idents(detail);
    let attention_ids = idents(attention);
    assert_ne!(explorer_ids, detail_ids);
    assert_ne!(explorer_ids, attention_ids);
    assert!(explorer_ids.contains("toggle:openrig-build/dev50"));
    for agent in ["dev50.driver", "dev50.guard", "dev50.qa"] {
        assert!(explorer_ids.contains(&format!("select:{agent}")));
    }
    assert!(detail_ids.contains("cross:driver-agent"));
    assert!(attention_ids.contains("attention:queue:q1"));
    assert!(attention_ids.contains("attention:queue:q2"));
    assert!(disclosure_before_the_row(explorer));
    let attention_rows = attention_ids
        .iter()
        .filter(|id| id.starts_with("attention:"))
        .count();
    assert_eq!(attention_rows, 2);
    explorer_ids
}

fn assert_rust_matches(explorer_ids: &BTreeSet<String>) {
    let rust_explorer = paint_idents(&expanded_pod());
    for id in [
        "toggle:openrig-build/dev50",
        "select:dev50.driver",
        "select:dev50.guard",
        "select:dev50.qa",
    ] {
        assert!(rust_explorer.contains(id), "rust explorer missing {id}");
        assert!(explorer_ids.contains(id), "corpus explorer missing {id}");
    }
    assert!(rust_disclosure_before_the_row(&paint_hits(&expanded_pod())));
    let mut detail_state = expanded_pod();
    detail_state.section = Section::Detail;
    detail_state.selection = Some(RowKey::Seat(seat_id("dev50.driver")));
    assert!(paint_idents(&detail_state).contains("cross:driver-agent"));
    let mut attention_state = expanded_pod();
    attention_state.section = Section::Attention;
    if let Some(seat) = attention_state.snapshot.seats.first_mut() {
        seat.state = SeatState::Attention(crewmux_api::ReadinessCode::LoginRequired);
    }
    assert!(paint_idents(&attention_state).contains("select:dev50.driver"));
}

fn load(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tui-hitmap")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("json {name}: {err}"))
}

fn idents(screen: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(hits) = screen.get("hits").and_then(Value::as_array) else {
        return out;
    };
    for hit in hits {
        let Some(identity) = hit.get("identity") else {
            continue;
        };
        let Some(kind) = identity.get("kind").and_then(Value::as_str) else {
            continue;
        };
        let Some(target) = identity.get("target").and_then(Value::as_str) else {
            continue;
        };
        if let Some(id) = canon(kind, target) {
            out.insert(id);
        }
    }
    out
}

fn canon(kind: &str, target: &str) -> Option<String> {
    match kind {
        "toggle" => {
            let rest = target.strip_prefix("pod:")?;
            let mut parts = rest.split('/');
            let _host = parts.next()?;
            let rig = parts.next()?;
            let pod = parts.next()?;
            if parts.next().is_some() {
                return None;
            }
            Some(format!("toggle:{rig}/{pod}"))
        }
        "select" => target
            .strip_prefix("agent:")
            .map(|name| format!("select:{name}"))
            .or_else(|| {
                target
                    .strip_prefix("pod:")
                    .map(|name| format!("select:pod:{name}"))
            }),
        "cross" => Some(format!("cross:{target}")),
        "attention" => Some(format!("attention:{target}")),
        _ => None,
    }
}

fn disclosure_before_the_row(screen: &Value) -> bool {
    let Some(hits) = screen.get("hits").and_then(Value::as_array) else {
        return false;
    };
    let toggle = hits.iter().position(|hit| {
        hit["identity"]["kind"] == "toggle"
            && hit["identity"]["target"] == "pod:vm-host/openrig-build/dev50"
    });
    let select = hits.iter().position(|hit| {
        hit["identity"]["kind"] == "select" && hit["identity"]["target"] == "pod:dev50"
    });
    let (Some(toggle_at), Some(select_at)) = (toggle, select) else {
        return false;
    };
    let Some(toggle_hit) = hits.get(toggle_at) else {
        return false;
    };
    toggle_at < select_at && toggle_hit["x1"] == toggle_hit["x2"]
}

fn rust_disclosure_before_the_row(hits: &HitMap) -> bool {
    let toggle = hits.targets.iter().position(|target| {
        matches!(
            &target.action,
            Action::ToggleDisclosure(RowKey::Pod { pod, .. }) if pod.as_str() == "dev50"
        )
    });
    let select = hits.targets.iter().position(|target| {
        matches!(
            &target.action,
            Action::Select(RowKey::Pod { pod, .. }) if pod.as_str() == "dev50"
        )
    });
    matches!((toggle, select), (Some(toggle_at), Some(select_at)) if toggle_at < select_at)
}

fn expanded_pod() -> State {
    let seats = ["dev50.driver", "dev50.guard", "dev50.qa"]
        .into_iter()
        .map(|name| SeatView {
            id: seat_id(name),
            name: SeatName::parse(name).expect("name"),
            rig_id: RigId::parse("openrig-build").expect("rig"),
            rig_name: RigName::parse("openrig-build").expect("rig name"),
            pod: PodName::parse("dev50").expect("pod"),
            state: SeatState::Ready,
            runtime: RuntimeKind::ClaudeCode,
            session: None,
            spec: Some(if name == "dev50.driver" {
                "driver-agent".to_owned()
            } else {
                "other-agent".to_owned()
            }),
        })
        .collect();
    let state = State::new(Size::new(120, 40), std::time::Instant::now());
    let (state, _) = update(
        state,
        crate::msg::Msg::Hydrated(Ok(Snapshot {
            seats,
            health: None,
            activity: vec![],
        })),
    );
    state
}

fn seat_id(name: &str) -> SeatId {
    SeatId::parse(name).expect("seat")
}

fn paint_hits(state: &State) -> HitMap {
    let mut terminal =
        Terminal::new(TestBackend::new(state.size.cols, state.size.rows)).expect("backend");
    let mut hits = HitMap::default();
    terminal
        .draw(|frame| {
            hits = view(state, frame);
        })
        .expect("draw");
    hits
}

fn paint_idents(state: &State) -> BTreeSet<String> {
    let hits = paint_hits(state);
    hits.targets
        .iter()
        .chain(hits.content.iter())
        .filter_map(|target| rust_identity(&target.action))
        .collect()
}

fn rust_identity(action: &Action) -> Option<String> {
    match action {
        Action::ToggleDisclosure(RowKey::Pod { rig, pod }) => {
            Some(format!("toggle:{}/{pod}", rig.as_str()))
        }
        Action::Select(RowKey::Seat(id)) => Some(format!("select:{}", id.as_str())),
        Action::Select(RowKey::Pod { pod, .. }) => Some(format!("select:pod:{}", pod.as_str())),
        Action::Cross(CrossLink::Spec { name }) => Some(format!("cross:{name}")),
        Action::Select(_)
        | Action::ToggleDisclosure(_)
        | Action::OpenSection(_)
        | Action::Attach(_)
        | Action::Scroll(_) => None,
    }
}
