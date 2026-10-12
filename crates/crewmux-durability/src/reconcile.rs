//! In-flight delivery reconciliation (ADR-0009).
//!
//! Queue rows are re-nudged with a pointer. Direct and startup deliveries are
//! acked, re-sent, or escalated from the transcript. Identical bodies are
//! counted by occurrence.

use crewmux_messaging::{QueueState, nudge_pointer};
use crewmux_transcript::{TranscriptCursor, TranscriptProbe, TurnMatch, match_turn, normalize_for};
use crewmux_types::{DeliveryId, QItemId, SeatId};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// `direct` send or a startup-file delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryChannel {
    /// `rig send`.
    Direct,
    /// A startup file.
    Startup,
}

/// One in-flight item restore has to account for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InFlightDelivery {
    /// A queue row. The body stays in the database.
    QueueRow {
        /// Item id.
        qitem: QItemId,
        /// Committed state. Restore does not change it.
        state: QueueState,
    },
    /// A direct or startup delivery.
    Direct {
        /// Delivery id.
        id: DeliveryId,
        /// Seat.
        seat: SeatId,
        /// `sha256(normalize(body))`.
        fingerprint: String,
        /// Body, used only to match the transcript.
        body: String,
        /// Absent when the crash was before the paste.
        delivered_at: Option<i64>,
        /// Transcript cursor at send time.
        cursor: Option<u64>,
        /// Channel. Both channels share this planner.
        channel: DeliveryChannel,
    },
}

/// What restore should do. None of these paste a queue body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileAction {
    /// Pointer nudge. Idempotent.
    ReNudge(QItemId),
    /// The transcript already has this delivery.
    Ack(DeliveryId),
    /// The transcript does not have it.
    ReSend(DeliveryId),
    /// A user turn contains the payload plus other text.
    Escalate(DeliveryId, String),
}

/// Plan reconciliation. Pure over `probe`.
///
/// # Errors
///
/// Returns [`Error::Probe`] when the transcript cannot be read.
pub fn plan_reconcile(
    inflight: &[InFlightDelivery],
    probe: &dyn TranscriptProbe,
) -> Result<Vec<ReconcileAction>, Error> {
    let mut actions = Vec::new();
    for item in inflight {
        if let InFlightDelivery::QueueRow { qitem, .. } = item {
            actions.push(ReconcileAction::ReNudge(qitem.clone()));
        }
    }
    let mut pending = Vec::new();
    for item in inflight {
        if let InFlightDelivery::Direct {
            id,
            seat,
            fingerprint,
            body,
            delivered_at,
            cursor,
            ..
        } = item
        {
            let Some(at) = delivered_at else {
                actions.push(ReconcileAction::ReSend(id.clone()));
                continue;
            };
            pending.push(Pending {
                id: id.clone(),
                seat: *seat,
                fingerprint: fingerprint.clone(),
                body: body.clone(),
                delivered_at: *at,
                cursor: *cursor,
            });
        }
    }
    pending.sort_by(|left, right| {
        left.delivered_at
            .cmp(&right.delivered_at)
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    });
    actions.extend(plan_pending(&pending, probe)?);
    Ok(actions)
}

/// Pointer text for a re-nudge. It does not contain the obligation body.
#[must_use]
pub fn renudge_text(id: &QItemId) -> String {
    nudge_pointer(id)
}

struct Pending {
    id: DeliveryId,
    seat: SeatId,
    fingerprint: String,
    body: String,
    delivered_at: i64,
    cursor: Option<u64>,
}

fn plan_pending(
    pending: &[Pending],
    probe: &dyn TranscriptProbe,
) -> Result<Vec<ReconcileAction>, Error> {
    let mut actions = Vec::new();
    for group in buckets(pending) {
        actions.extend(plan_bucket(&group, probe)?);
    }
    Ok(actions)
}

/// Every pending row with this seat and fingerprint, including ones a different
/// body sat between. Adjacent-only grouping acks both copies of X in X, Y, X.
fn buckets(pending: &[Pending]) -> Vec<Vec<&Pending>> {
    let mut keys = Vec::new();
    let mut groups: Vec<Vec<&Pending>> = Vec::new();
    for item in pending {
        let found =
            keys.iter()
                .position(|(seat, fingerprint): &(crewmux_types::SeatId, String)| {
                    *seat == item.seat && fingerprint == &item.fingerprint
                });
        if let Some(index) = found {
            if let Some(group) = groups.get_mut(index) {
                group.push(item);
            }
            continue;
        }
        keys.push((item.seat, item.fingerprint.clone()));
        groups.push(vec![item]);
    }
    groups
}

fn plan_bucket(
    group: &[&Pending],
    probe: &dyn TranscriptProbe,
) -> Result<Vec<ReconcileAction>, Error> {
    let Some(first) = group.first() else {
        return Ok(Vec::new());
    };
    let floor = group_floor(group);
    let turns = probe
        .user_turns_after(&first.seat, Some(TranscriptCursor(floor)))
        .map_err(|err| Error::Probe(err.to_string()))?;
    let classified = classify(probe.runtime(&first.seat), &first.body, &turns);
    let mut used = vec![false; classified.len()];
    let mut actions = Vec::new();
    for item in group {
        actions.push(action_for(item, &classified, &mut used));
    }
    Ok(actions)
}

fn group_floor(group: &[&Pending]) -> u64 {
    if group.iter().any(|item| item.cursor.is_none()) {
        return 0;
    }
    group
        .iter()
        .filter_map(|item| item.cursor)
        .min()
        .unwrap_or(0)
}

fn classify(
    runtime: crewmux_types::RuntimeKind,
    body: &str,
    turns: &[crewmux_transcript::UserTurn],
) -> Vec<(u64, TurnMatch)> {
    let mut classified = Vec::new();
    for turn in turns {
        let kind = relate(runtime, body, &turn.text);
        if kind != TurnMatch::Different {
            classified.push((turn.at.0, kind));
        }
    }
    classified
}

fn action_for(item: &Pending, turns: &[(u64, TurnMatch)], used: &mut [bool]) -> ReconcileAction {
    let floor = item.cursor.unwrap_or(0);
    if take_equal(turns, used, floor) {
        return ReconcileAction::Ack(item.id.clone());
    }
    if turns
        .iter()
        .any(|(at, kind)| *at >= floor && *kind == TurnMatch::Composite)
    {
        return ReconcileAction::Escalate(item.id.clone(), "contaminated user turn".to_owned());
    }
    ReconcileAction::ReSend(item.id.clone())
}

fn take_equal(turns: &[(u64, TurnMatch)], used: &mut [bool], floor: u64) -> bool {
    for (index, (at, kind)) in turns.iter().enumerate() {
        if *at < floor || *kind != TurnMatch::Equal {
            continue;
        }
        let Some(slot) = used.get_mut(index) else {
            continue;
        };
        if *slot {
            continue;
        }
        *slot = true;
        return true;
    }
    false
}

fn relate(runtime: crewmux_types::RuntimeKind, body: &str, turn: &str) -> TurnMatch {
    match match_turn(runtime, body, turn) {
        TurnMatch::Equal => TurnMatch::Equal,
        TurnMatch::Composite => TurnMatch::Composite,
        TurnMatch::Different => {
            let sent = normalize_for(runtime, body);
            let stored = normalize_for(runtime, turn);
            if !sent.is_empty() && stored != sent && stored.contains(&sent) {
                TurnMatch::Composite
            } else {
                TurnMatch::Different
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeliveryChannel, InFlightDelivery, ReconcileAction, plan_reconcile, renudge_text};
    use crewmux_messaging::{QueueState, leaks_body};
    use crewmux_transcript::{TranscriptCursor, TranscriptProbe, UserTurn, fingerprint};
    use crewmux_types::{DeliveryId, QItemId, RuntimeKind, SeatId};
    struct MapProbe {
        seat: SeatId,
        turns: Vec<UserTurn>,
        runtime: RuntimeKind,
    }

    impl TranscriptProbe for MapProbe {
        fn runtime(&self, _seat: &SeatId) -> RuntimeKind {
            self.runtime
        }

        fn user_turns_after(
            &self,
            seat: &SeatId,
            after: Option<TranscriptCursor>,
        ) -> Result<Vec<UserTurn>, crewmux_transcript::Error> {
            if seat != &self.seat {
                return Ok(Vec::new());
            }
            let min = after.map_or(0, |cursor| cursor.0);
            Ok(self
                .turns
                .iter()
                .filter(|turn| turn.at.0 >= min)
                .cloned()
                .collect())
        }
    }

    fn direct(
        id: &str,
        seat: SeatId,
        body: &str,
        delivered: Option<i64>,
        cursor: Option<u64>,
    ) -> InFlightDelivery {
        InFlightDelivery::Direct {
            id: DeliveryId::new(id).expect("id"),
            seat,
            fingerprint: fingerprint(RuntimeKind::Stub, body).to_string(),
            body: body.to_owned(),
            delivered_at: delivered,
            cursor,
            channel: DeliveryChannel::Direct,
        }
    }

    fn turn(text: &str, at: u64) -> UserTurn {
        UserTurn {
            text: text.to_owned(),
            at: TranscriptCursor(at),
        }
    }

    #[test]
    fn queue_rows_only_renudged() {
        let id = QItemId::new("baton-1").expect("id");
        let body = "secret-body-token";
        let actions = plan_reconcile(
            &[InFlightDelivery::QueueRow {
                qitem: id.clone(),
                state: QueueState::HandedOff,
            }],
            &MapProbe {
                seat: SeatId::new(),
                turns: Vec::new(),
                runtime: RuntimeKind::Stub,
            },
        )
        .expect("plan");
        assert_eq!(actions, vec![ReconcileAction::ReNudge(id.clone())]);
        let text = renudge_text(&id);
        assert!(!leaks_body(&text, body), "{text}");
    }

    #[test]
    fn no_duplicates_after_crash_between_paste_and_ack() {
        let seat = SeatId::new();
        let mut items = Vec::new();
        let mut turns = Vec::new();
        for index in 0..18 {
            let id = format!("d{index}");
            let body = format!("message-{index}");
            let phase = index % 3;
            if phase == 0 {
                items.push(direct(&id, seat, &body, None, Some(0)));
            } else {
                items.push(direct(&id, seat, &body, Some(i64::from(index)), Some(0)));
                turns.push(turn(&body, u64::try_from(index).unwrap_or(0)));
                if phase == 2 {
                    items.pop();
                }
            }
        }
        let actions = plan_reconcile(
            &items,
            &MapProbe {
                seat,
                turns,
                runtime: RuntimeKind::Stub,
            },
        )
        .expect("plan");
        let mut resend = 0;
        let mut ack = 0;
        for action in &actions {
            match action {
                ReconcileAction::ReSend(_) => resend += 1,
                ReconcileAction::Ack(_) => ack += 1,
                ReconcileAction::ReNudge(_) | ReconcileAction::Escalate(_, _) => {
                    panic!("unexpected {action:?}")
                }
            }
        }
        assert_eq!(resend, 6, "before paste");
        assert_eq!(ack, 6, "between paste and ack");
    }

    #[test]
    fn identical_bodies_counted_by_occurrence() {
        let seat = SeatId::new();
        let probe = MapProbe {
            seat,
            turns: vec![turn("same", 10)],
            runtime: RuntimeKind::Stub,
        };
        let actions = plan_reconcile(
            &[
                direct("older", seat, "same", Some(1), Some(0)),
                direct("newer", seat, "same", Some(2), Some(0)),
            ],
            &probe,
        )
        .expect("plan");
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, ReconcileAction::Ack(_)))
        );
        let resends: Vec<_> = actions
            .iter()
            .filter(|action| matches!(action, ReconcileAction::ReSend(_)))
            .collect();
        assert_eq!(resends.len(), 1);
    }

    #[test]
    fn contaminated_turn_escalates() {
        let seat = SeatId::new();
        let probe = MapProbe {
            seat,
            turns: vec![turn("payload plus extra", 1)],
            runtime: RuntimeKind::Stub,
        };
        let actions = plan_reconcile(&[direct("c", seat, "payload", Some(1), Some(0))], &probe)
            .expect("plan");
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions.first(),
            Some(ReconcileAction::Escalate(_, _))
        ));
    }

    #[test]
    fn normalisation_matches_crlf_nfc_and_trailing_space_only() {
        let seat = SeatId::new();
        let cases = [
            ("hello\n", "hello\r\n", true),
            ("café", "cafe\u{0301}", true),
            ("hello  ", "hello", true),
            ("hello world", "hello  world", false),
        ];
        for (sent, stored, expect_ack) in cases {
            let probe = MapProbe {
                seat,
                turns: vec![turn(stored, 1)],
                runtime: RuntimeKind::Stub,
            };
            let actions =
                plan_reconcile(&[direct("n", seat, sent, Some(1), Some(0))], &probe).expect("plan");
            let acked = matches!(actions.first(), Some(ReconcileAction::Ack(_)));
            assert_eq!(acked, expect_ack, "{sent:?} vs {stored:?} {actions:?}");
        }
    }

    #[test]
    fn startup_channel_uses_the_same_rule() {
        let seat = SeatId::new();
        let mut item = direct("s", seat, "boot", None, Some(0));
        if let InFlightDelivery::Direct { channel, .. } = &mut item {
            *channel = DeliveryChannel::Startup;
        }
        let actions = plan_reconcile(
            &[item],
            &MapProbe {
                seat,
                turns: Vec::new(),
                runtime: RuntimeKind::Stub,
            },
        )
        .expect("plan");
        assert!(matches!(actions.first(), Some(ReconcileAction::ReSend(_))));
    }

    #[test]
    fn interleaved_copies_are_counted_once_per_occurrence() {
        let seat = SeatId::new();
        let probe = MapProbe {
            seat,
            turns: vec![turn("X", 5)],
            runtime: RuntimeKind::Stub,
        };
        let actions = plan_reconcile(
            &[
                direct("x1", seat, "X", Some(1), Some(0)),
                direct("y1", seat, "Y", Some(2), Some(0)),
                direct("x2", seat, "X", Some(3), Some(0)),
            ],
            &probe,
        )
        .expect("plan");
        let ack = actions.iter().filter(|action| match action {
            ReconcileAction::Ack(id) => id.as_str() == "x1",
            ReconcileAction::ReSend(_)
            | ReconcileAction::ReNudge(_)
            | ReconcileAction::Escalate(_, _) => false,
        });
        assert_eq!(ack.count(), 1, "{actions:?}");
        assert!(actions.iter().any(|action| matches!(
            action,
            ReconcileAction::ReSend(id) if id.as_str() == "x2"
        )));
        assert!(actions.iter().any(|action| matches!(
            action,
            ReconcileAction::ReSend(id) if id.as_str() == "y1"
        )));
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, ReconcileAction::Ack(_)))
                .count(),
            1,
            "{actions:?}"
        );
    }

    #[test]
    fn each_row_consumes_a_turn_after_its_own_cursor() {
        let seat = SeatId::new();
        let probe = MapProbe {
            seat,
            turns: vec![turn("X", 50)],
            runtime: RuntimeKind::Stub,
        };
        let actions = plan_reconcile(
            &[
                direct("late", seat, "X", Some(1), Some(100)),
                direct("early", seat, "X", Some(2), Some(0)),
            ],
            &probe,
        )
        .expect("plan");
        assert!(actions.iter().any(|action| matches!(
            action,
            ReconcileAction::ReSend(id) if id.as_str() == "late"
        )));
        assert!(actions.iter().any(|action| matches!(
            action,
            ReconcileAction::Ack(id) if id.as_str() == "early"
        )));
    }
}
