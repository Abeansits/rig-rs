//! Ports of the v1 TypeScript TUI tests named in the ledger.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crewmux_api::{RigId, StreamEvent, StreamKind};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::action::{Action, HitMap, HitTarget};
use crate::input::{
    Dispatch, HitAction, MouseAction, Perform, decode_mice, dispatch_sgr, resolve_mouse_action,
    sgr_click,
};
use crate::msg::{Cmd, KeyEvent, KeyKind, Msg};
use crate::refresh::SingleFlight;
use crate::sse::{Opened, connect_loop};
use crate::state::{ConnState, LoadState, RowKey, State};
use crate::update::update;

#[tokio::test]
async fn coalesces_overlapping_refresh_into_one_trailing_pass() {
    let calls = Arc::new(AtomicUsize::new(0));
    let commits = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = tokio::sync::oneshot::channel::<&'static str>();
    let rx = Arc::new(Mutex::new(Some(rx)));
    let gate = SingleFlight::new({
        let calls = Arc::clone(&calls);
        let commits = Arc::clone(&commits);
        let rx = Arc::clone(&rx);
        move || {
            let calls = Arc::clone(&calls);
            let commits = Arc::clone(&commits);
            let rx = Arc::clone(&rx);
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                let snapshot = if n == 1 {
                    let receiver = rx.lock().take().ok_or_else(|| Arc::from("missing gate"))?;
                    receiver.await.map_err(|_| Arc::from("dropped"))?
                } else {
                    "item-6"
                };
                commits.lock().push(snapshot);
                Ok(())
            }
        }
    });
    let tracker = TaskTracker::new();
    let cancel = CancellationToken::new();
    let mut first = gate.request(&tracker, cancel.clone());
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let mut overlap = gate.request(&tracker, cancel.clone());
    assert_eq!(first.id, overlap.id);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tx.send("item-7").expect("first read");
    first.wait().await.expect("first flight");
    overlap.wait().await.expect("overlap flight");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(commits.lock().as_slice(), ["item-7", "item-6"]);
    let mut quiet = gate.request(&tracker, cancel.clone());
    quiet.wait().await.expect("quiet flight");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    cancel.cancel();
    tracker.close();
    tracker.wait().await;
}

#[tokio::test]
async fn releases_the_guard_after_rejection() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let gate = SingleFlight::new({
        let attempts = Arc::clone(&attempts);
        move || {
            let attempts = Arc::clone(&attempts);
            async move {
                let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 {
                    Err(Arc::from("hydrate failed"))
                } else {
                    Ok(())
                }
            }
        }
    });
    let tracker = TaskTracker::new();
    let cancel = CancellationToken::new();
    let mut first = gate.request(&tracker, cancel.clone());
    let err = first.wait().await.expect_err("first rejection");
    assert!(err.contains("hydrate failed"));
    let mut second = gate.request(&tracker, cancel.clone());
    second.wait().await.expect("retry");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    cancel.cancel();
    tracker.close();
    tracker.wait().await;
}

#[test]
fn wheel_scroll_keeps_click_coordinates() {
    let mice = decode_mice("\u{1b}[<65;10;5M");
    assert_eq!(mice.len(), 1);
    assert_eq!(mice[0].button, 65);
    assert_eq!(mice[0].x, 10);
    assert_eq!(mice[0].y, 5);
    let click = decode_mice(&sgr_click(10, 5));
    assert_eq!(click.len(), 1);
    assert_eq!(click[0].button, 0);
}

#[test]
fn wheel_scrolls_the_pane_under_the_pointer() {
    let explorer = resolve_mouse_action(65, 5, 30, 20);
    assert_eq!(
        explorer,
        Some(MouseAction::Select {
            delta: 3,
            row_count: 20
        })
    );
    let content = resolve_mouse_action(64, 90, 30, 20);
    assert_eq!(content, Some(MouseAction::ContentScroll { delta: -3 }));
}

#[test]
fn primary_click_dispatch_ignores_other_buttons() {
    let open = HitAction::Open {
        view: "rig:example".to_owned(),
    };
    let seq = |button| format!("\u{1b}[<{button};5;3M");
    for button in [1_u16, 2] {
        assert!(matches!(
            dispatch_sgr(&seq(button), &open, false, 20, 0),
            Dispatch::None
        ));
    }
    assert!(matches!(
        dispatch_sgr(&seq(0), &open, false, 20, 0),
        Dispatch::Perform(Perform::Open { .. })
    ));
    assert!(matches!(
        dispatch_sgr(&seq(64), &open, false, 20, 0),
        Dispatch::Perform(Perform::Select { delta: -3, .. })
    ));
}

#[test]
fn load_state_is_honest() {
    let mut load = LoadState::initial();
    assert!(!load.shows_live());
    load.connection = ConnState::Connected;
    assert!(load.shows_live());
    load.stale = true;
    assert!(!load.shows_live());
    load.connection = ConnState::Dropped;
    assert_eq!(load.connection, ConnState::Dropped);
    assert!(load.stale);
    load.connection = ConnState::Unavailable;
    assert_eq!(load.connection, ConnState::Unavailable);
}

#[test]
fn disclosure_fixture_matches_the_hit_map() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tui-hitmap/disclosure.json");
    let text = std::fs::read_to_string(path).expect("fixture");
    let value: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert!(
        value
            .get("shift_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("ADR-0004")
    );
    let key = RowKey::Rig(RigId::parse("rig-1").expect("rig"));
    let hits = HitMap {
        targets: vec![
            HitTarget {
                y: 2,
                x1: 1,
                x2: 1,
                action: Action::ToggleDisclosure(key.clone()),
            },
            HitTarget {
                y: 2,
                x1: 1,
                x2: 30,
                action: Action::Select(key),
            },
        ],
        content: vec![],
    };
    let points = value
        .get("points")
        .and_then(|v| v.as_array())
        .expect("points");
    for point in points {
        let x = u16::try_from(
            point
                .get("x")
                .and_then(serde_json::Value::as_u64)
                .expect("x"),
        )
        .expect("x");
        let y = u16::try_from(
            point
                .get("y")
                .and_then(serde_json::Value::as_u64)
                .expect("y"),
        )
        .expect("y");
        let action = point
            .get("action")
            .and_then(|v| v.as_str())
            .expect("action");
        match hits.hit(x, y) {
            Some(Action::ToggleDisclosure(_)) => assert_eq!(action, "toggle"),
            Some(Action::Select(_)) => assert_eq!(action, "select"),
            other => panic!("unexpected hit {other:?}"),
        }
    }
}

#[test]
fn disclosure_hit_beats_the_row() {
    let key = RowKey::Rig(RigId::parse("rig-1").expect("rig"));
    let hits = HitMap {
        targets: vec![
            HitTarget {
                y: 2,
                x1: 1,
                x2: 1,
                action: Action::ToggleDisclosure(key.clone()),
            },
            HitTarget {
                y: 2,
                x1: 1,
                x2: 30,
                action: Action::Select(key),
            },
        ],
        content: vec![],
    };
    assert!(matches!(hits.hit(1, 2), Some(Action::ToggleDisclosure(_))));
    assert!(matches!(hits.hit(2, 2), Some(Action::Select(_))));
}

proptest::proptest! {
    #[test]
    fn update_is_pure(step in 0u8..8) {
        let state = State::new(crate::state::Size::new(80, 24), std::time::Instant::now());
        let msg = match step % 4 {
            0 => Msg::Tick,
            1 => Msg::Key(KeyEvent { kind: KeyKind::Char('j') }),
            2 => Msg::Resize(100, 30),
            _ => Msg::Conn(ConnState::Connected),
        };
        let (left, left_cmds) = update(state.clone(), msg.clone());
        let (right, right_cmds) = update(state, msg);
        assert_eq!(left.section, right.section);
        assert_eq!(left.scroll, right.scroll);
        assert_eq!(left.load, right.load);
        assert_eq!(left_cmds.len(), right_cmds.len());
        for (lhs, rhs) in left_cmds.iter().zip(right_cmds.iter()) {
            assert!(cmd_eq(lhs, rhs));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn backs_off_until_notifications_resume() {
    use std::time::Duration;
    let arrivals = Arc::new(Mutex::new(Vec::<Duration>::new()));
    let notes = Arc::new(Mutex::new(Vec::<String>::new()));
    let count = Arc::new(AtomicUsize::new(0));
    let start = tokio::time::Instant::now();
    let cancel = CancellationToken::new();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
    let (status_tx, mut status_rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move { while status_rx.recv().await.is_some() {} });
    let open_cancel = cancel.clone();
    let arrivals_open = Arc::clone(&arrivals);
    let count_open = Arc::clone(&count);
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        connect_loop(
            task_cancel,
            Duration::from_millis(80),
            move |_last_id| {
                let n = count_open.fetch_add(1, Ordering::SeqCst) + 1;
                arrivals_open.lock().push(start.elapsed());
                let open_cancel = open_cancel.clone();
                async move {
                    if n == 5 {
                        Opened::Chunks(Box::pin(futures_util::stream::unfold(
                            open_cancel,
                            |token| async move {
                                token.cancelled().await;
                                None
                            },
                        )))
                    } else if n == 4 {
                        chunk(b"data: {\"type\":\"seat.activity_changed\",\"seq\":7}\n\n")
                    } else {
                        chunk(b": connected\n\n")
                    }
                }
            },
            event_tx,
            status_tx,
        )
        .await;
    });
    for delay in [0_u64, 80, 160, 320, 80] {
        if delay > 0 {
            tokio::time::advance(Duration::from_millis(delay)).await;
        }
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }
    }
    let mut seen = Vec::new();
    while let Ok(value) = event_rx.try_recv() {
        seen.push(value.to_string());
    }
    notes.lock().extend(seen);
    assert_eq!(
        arrivals.lock().as_slice(),
        &[
            Duration::ZERO,
            Duration::from_millis(80),
            Duration::from_millis(240),
            Duration::from_millis(560),
            Duration::from_millis(640),
        ]
    );
    let parsed: serde_json::Value =
        serde_json::from_str(notes.lock().first().expect("event")).expect("json");
    assert_eq!(
        parsed.get("type").and_then(|v| v.as_str()),
        Some("seat.activity_changed")
    );
    assert_eq!(
        parsed.get("seq").and_then(serde_json::Value::as_u64),
        Some(7)
    );
    cancel.cancel();
    tokio::time::advance(Duration::from_secs(30)).await;
    task.await.expect("loop");
    drain.abort();
    assert_eq!(arrivals.lock().len(), 5);
}

#[test]
fn gap_event_requests_a_full_refresh() {
    let state = State::new(crate::state::Size::new(80, 24), std::time::Instant::now());
    let event = StreamEvent {
        id: 9,
        kind: StreamKind::Gap,
        payload: None,
    };
    let (next, cmds) = update(state, Msg::Live(event));
    assert!(next.load.stale);
    assert!(next.load.in_flight);
    assert_eq!(cmds, vec![Cmd::Hydrate]);
}

#[tokio::test(start_paused = true)]
async fn reconnect_passes_the_last_event_id() {
    use std::time::Duration;
    let seen = Arc::new(Mutex::new(Vec::<Option<u64>>::new()));
    let count = Arc::new(AtomicUsize::new(0));
    let cancel = CancellationToken::new();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
    let (status_tx, mut status_rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move { while status_rx.recv().await.is_some() {} });
    let open_cancel = cancel.clone();
    let seen_open = Arc::clone(&seen);
    let count_open = Arc::clone(&count);
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        connect_loop(
            task_cancel,
            Duration::from_millis(80),
            move |last_id| {
                seen_open.lock().push(last_id);
                let n = count_open.fetch_add(1, Ordering::SeqCst) + 1;
                let open_cancel = open_cancel.clone();
                async move {
                    if n == 1 {
                        chunk(
                            b"id: 4\ndata: {\"id\":4,\"type\":\"seat_died\",\"payload\":{\"seat\":\"s1\"}}\n\n",
                        )
                    } else {
                        Opened::Chunks(Box::pin(futures_util::stream::unfold(
                            open_cancel,
                            |token| async move {
                                token.cancelled().await;
                                None
                            },
                        )))
                    }
                }
            },
            event_tx,
            status_tx,
        )
        .await;
    });
    for delay in [0_u64, 80] {
        if delay > 0 {
            tokio::time::advance(Duration::from_millis(delay)).await;
        }
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }
    }
    let mut notes = Vec::new();
    while let Ok(value) = event_rx.try_recv() {
        notes.push(value);
    }
    assert_eq!(
        notes
            .first()
            .and_then(|value| value.get("id"))
            .and_then(serde_json::Value::as_u64),
        Some(4)
    );
    assert_eq!(seen.lock().as_slice(), &[None, Some(4)]);
    cancel.cancel();
    tokio::time::advance(Duration::from_secs(30)).await;
    task.await.expect("loop");
    drain.abort();
}

fn chunk(bytes: &'static [u8]) -> Opened {
    Opened::Chunks(Box::pin(futures_util::stream::iter([Ok(bytes.to_vec())])))
}

#[expect(
    clippy::match_same_arms,
    reason = "each command pair is its own equality case"
)]
fn cmd_eq(left: &Cmd, right: &Cmd) -> bool {
    match (left, right) {
        (Cmd::Hydrate, Cmd::Hydrate) => true,
        (Cmd::Quit, Cmd::Quit) => true,
        (Cmd::Reply(a, av), Cmd::Reply(b, bv)) => a == b && av == bv,
        (Cmd::Attach(_), Cmd::Attach(_)) => true,
        _ => false,
    }
}
