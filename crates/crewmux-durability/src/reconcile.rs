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
    let mut index = 0;
    while index < pending.len() {
        let end = group_end(&pending, index);
        let Some(group) = pending.get(index..end) else {
            break;
        };
        actions.extend(plan_group(group, probe)?);
        index = end;
    }
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

fn group_end(pending: &[Pending], start: usize) -> usize {
    let Some(first) = pending.get(start) else {
        return start;
    };
    let mut end = start.saturating_add(1);
    while pending
        .get(end)
        .is_some_and(|item| item.seat == first.seat && item.fingerprint == first.fingerprint)
    {
        end = end.saturating_add(1);
    }
    end
}

fn plan_group(
    group: &[Pending],
    probe: &dyn TranscriptProbe,
) -> Result<Vec<ReconcileAction>, Error> {
    let Some(first) = group.first() else {
        return Ok(Vec::new());
    };
    let cursor = group.iter().filter_map(|item| item.cursor).min();
    let after = cursor.map(TranscriptCursor);
    let turns = probe
        .user_turns_after(&first.seat, after)
        .map_err(|err| Error::Probe(err.to_string()))?;
    let runtime = probe.runtime(&first.seat);
    let mut equal = 0usize;
    let mut composite = false;
    for turn in &turns {
        match relate(runtime, &first.body, &turn.text) {
            TurnMatch::Equal => equal = equal.saturating_add(1),
            TurnMatch::Composite => composite = true,
            TurnMatch::Different => {}
        }
    }
    let mut actions = Vec::new();
    for (index, item) in group.iter().enumerate() {
        if index < equal {
            actions.push(ReconcileAction::Ack(item.id.clone()));
        } else if composite {
            actions.push(ReconcileAction::Escalate(
                item.id.clone(),
                "contaminated user turn".to_owned(),
            ));
        } else {
            actions.push(ReconcileAction::ReSend(item.id.clone()));
        }
    }
    Ok(actions)
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
}
