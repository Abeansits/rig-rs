//! Guarded direct send. The nudge path uses the same verdict.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use crewmux_adapters::{Readiness, classify};
use crewmux_core::DomainEvent;
use crewmux_messaging::{
    DeliveryOutcome, DeliveryReceipt, OutboxRecord, SessionRef, obligation_fingerprint, wrap_pane,
};
use crewmux_mux::{
    CaptureRange, ComposerState, GuardConfig, GuardVerdict, Key, PasteOpts, evaluate,
};
use crewmux_types::ReadinessCode;
use serde_json::{Value, json};

use crate::composer::{ComposerParser, classify_stub, stub_drafting};
use crate::error::Error;
use crate::launch::now_ts;
use crate::notify::emit;
use crate::paste_check::{PasteHit, classify_paste, clear_composer, history_range, pasted_screen};
use crate::query::{address_of, live_seats, pane_of};
use crate::state::App;

const REASON_REQUIRED: &str = "--dangerously-interact requires --reason explaining why the prompt is being driven. No text was sent.";

/// One `rig send` or `rig broadcast`.
pub(crate) struct Outbound {
    pub from: String,
    pub targets: Vec<String>,
    pub rig: Option<String>,
    pub all: bool,
    pub body: String,
    pub interact: bool,
    pub reason: Option<String>,
    /// Retained so `rig send --verify` stays on the wire. Rendering is required
    /// for every send, whether or not the flag is set.
    pub verify: bool,
}

pub(crate) async fn deliver(app: &App, outbound: Outbound) -> Result<Value, Error> {
    let seats = resolve(app, &outbound).await?;
    if seats.is_empty() {
        return Err(Error::Message("no seats to send to".to_owned()));
    }
    let mut rows = Vec::new();
    let mut accepted = 0u32;
    let mut failed = 0u32;
    for seat in seats {
        match one(app, &outbound, &seat).await {
            Ok(row) => {
                if row_failed(&row) {
                    failed = failed.saturating_add(1);
                } else {
                    accepted = accepted.saturating_add(1);
                }
                rows.push(row);
            }
            Err(err) => {
                failed = failed.saturating_add(1);
                rows.push(json!({"seat": seat, "error": err.to_string()}));
            }
        }
    }
    let stored = rows.iter().any(|row| row.get("outcome").is_some());
    if !stored {
        let message = rows
            .first()
            .and_then(|row| row.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("send failed")
            .to_owned();
        return Err(Error::Message(message));
    }
    let status = if failed == 0 {
        "ok"
    } else if accepted == 0 {
        "failed"
    } else {
        "partial"
    };
    Ok(json!({"status": status, "receipts": rows}))
}

async fn resolve(app: &App, outbound: &Outbound) -> Result<Vec<String>, Error> {
    if outbound.all {
        return names(app, None).await;
    }
    if let Some(rig) = &outbound.rig {
        return names(app, Some(rig.as_str())).await;
    }
    Ok(outbound.targets.clone())
}

async fn names(app: &App, rig: Option<&str>) -> Result<Vec<String>, Error> {
    let seats = live_seats(app).await?;
    let mut out = Vec::new();
    for seat in seats {
        let address = address_of(app, &seat).await?;
        if let Some(rig) = rig {
            let name = address.split_once('@').map_or("", |(_, host)| host);
            if name != rig {
                continue;
            }
        }
        out.push(address);
    }
    Ok(out)
}

async fn one(app: &App, outbound: &Outbound, target: &str) -> Result<Value, Error> {
    let seat = crate::query::find_address(app, target).await?;
    let address = address_of(app, &seat).await?;
    let pane = pane_of(&seat)?;
    let shot = app.mux.capture(&pane, CaptureRange::Visible).await?;
    let readiness = classify(&shot.text);
    if readiness != Readiness::Ready {
        return not_ready(app, outbound, &address, &pane, &shot.text, &readiness).await;
    }
    guarded(app, outbound, &address, &pane, true).await
}

async fn not_ready(
    app: &App,
    outbound: &Outbound,
    address: &str,
    pane: &crewmux_mux::PaneRef,
    _screen: &str,
    readiness: &Readiness,
) -> Result<Value, Error> {
    if !outbound.interact {
        return Err(Error::Message(refusal(address, &reason_text(readiness))));
    }
    let Some(reason) = outbound
        .reason
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    else {
        return Err(Error::Message(REASON_REQUIRED.to_owned()));
    };
    paste_and_check(app, outbound, address, pane, false, reason).await
}

async fn guarded(
    app: &App,
    outbound: &Outbound,
    address: &str,
    pane: &crewmux_mux::PaneRef,
    bracketed: bool,
) -> Result<Value, Error> {
    let now = Instant::now();
    let first = verdict(app, address, pane, now).await?;
    if !matches!(first.0.verdict, GuardVerdict::Deliver) {
        return receipt_row(app, outbound, address, &first.0, None).await;
    }
    let second = verdict(app, address, pane, Instant::now()).await?;
    if !matches!(second.0.verdict, GuardVerdict::Deliver) {
        return receipt_row(app, outbound, address, &second.0, None).await;
    }
    paste_and_check(app, outbound, address, pane, bracketed, "").await
}

async fn verdict(
    app: &App,
    address: &str,
    pane: &crewmux_mux::PaneRef,
    now: Instant,
) -> Result<(crewmux_mux::Evaluated, String), Error> {
    let mut inputs = app.mux.guard_inputs(pane, 40).await?;
    inputs.last_human_keystroke_tap = app
        .taps
        .last(address)
        .or_else(|| app.taps.last(seat_name(address)));
    let age = app
        .drafts
        .observe(address, stub_drafting(&inputs.tail.text), now)
        .await;
    let composer = classify_stub(&inputs.tail.text, age);
    let cfg = GuardConfig {
        quiet: Duration::from_secs(3),
        max_defer: Duration::from_secs(30),
        wall: SystemTime::now(),
    };
    let evaluated = evaluate(&inputs, composer, &cfg, now);
    Ok((evaluated, inputs.tail.text))
}

async fn paste_and_check(
    app: &App,
    outbound: &Outbound,
    address: &str,
    pane: &crewmux_mux::PaneRef,
    bracketed: bool,
    interact_reason: &str,
) -> Result<Value, Error> {
    let payload = wrap_pane(&outbound.from, address, &outbound.body);
    // History, joined. The composer region of this capture is the draft at
    // verdict time; rows above the prompt are not part of the comparison.
    let range = history_range(&payload);
    let before = app.mux.capture(pane, range).await?;
    let opts = PasteOpts {
        bracketed,
        preserve_lf: true,
    };
    tracing::debug!(verify = outbound.verify, seat = address, "paste");
    app.mux.paste(pane, &payload, opts).await?;
    let after = pasted_screen(app, pane, &before.text, &payload, range).await?;
    let mut degraded = ComposerParser::Stub.state(&before.text, None) == ComposerState::Unknown;
    let lines = payload.lines().count();
    let outcome = match classify_paste(&before.text, &after, &payload) {
        PasteHit::Clean => {
            app.mux.send_keys(pane, &[Key::Enter]).await?;
            DeliveryOutcome::Delivered
        }
        PasteHit::Contaminated => {
            if !clear_composer(app, pane, range, lines).await? {
                degraded = true;
            }
            raise(app, address, "contaminated").await;
            DeliveryOutcome::Contaminated
        }
        PasteHit::Missing => {
            if !clear_composer(app, pane, range, lines).await? {
                degraded = true;
            }
            raise(app, address, "did not render").await;
            DeliveryOutcome::Failed
        }
    };
    stored(app, outbound, address, outcome, degraded, interact_reason).await
}

async fn receipt_row(
    app: &App,
    outbound: &Outbound,
    address: &str,
    evaluated: &crewmux_mux::Evaluated,
    interact_reason: Option<&str>,
) -> Result<Value, Error> {
    let outcome = match evaluated.verdict {
        GuardVerdict::Escalate(_) => DeliveryOutcome::Escalated,
        GuardVerdict::Deliver => DeliveryOutcome::Delivered,
        GuardVerdict::Defer { .. } => DeliveryOutcome::Deferred,
    };
    stored(
        app,
        outbound,
        address,
        outcome,
        evaluated.guard_degraded,
        interact_reason.unwrap_or(""),
    )
    .await
}

async fn stored(
    app: &App,
    outbound: &Outbound,
    address: &str,
    outcome: DeliveryOutcome,
    guard_degraded: bool,
    interact_reason: &str,
) -> Result<Value, Error> {
    let cursor = app.cursor.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let fingerprint = obligation_fingerprint(&outbound.from, address, &outbound.body);
    let receipt = DeliveryReceipt {
        outcome,
        guard_degraded,
        fingerprint: fingerprint.clone(),
        cursor,
        at: now_ts(),
        interact_reason: (!interact_reason.is_empty()).then(|| interact_reason.to_owned()),
    };
    let doc = serde_json::to_string(&receipt).map_err(|err| Error::Message(err.to_string()))?;
    let store = Arc::clone(&app.store);
    let inserted = tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::insert_receipt(conn, cursor, &doc))
    })
    .await
    .map_err(crate::state::join_err)?;
    inserted.map_err(|err| Error::Message(format!("receipt: {err}")))?;
    if let Ok(destination) = SessionRef::new(address) {
        crate::inbox_http::record(
            app,
            OutboxRecord {
                qitem: None,
                destination,
                body: outbound.body.clone(),
                at: now_ts(),
            },
        )
        .await;
    }
    emit(
        app,
        DomainEvent::DeliveryRecorded {
            seat: address.to_owned(),
            outcome: outcome_name(outcome).to_owned(),
            at: now_ts(),
        },
    )
    .await;
    Ok(json!({
        "seat": address,
        "outcome": outcome_name(outcome),
        "guard_degraded": guard_degraded,
        "fingerprint": fingerprint,
        "cursor": cursor,
        "bracketed": interact_reason.is_empty(),
        "interact_reason": receipt.interact_reason,
    }))
}

fn row_failed(row: &Value) -> bool {
    matches!(
        row.get("outcome").and_then(Value::as_str),
        Some("contaminated" | "failed")
    )
}

async fn raise(app: &App, address: &str, reason: &str) {
    emit(
        app,
        DomainEvent::AttentionRaised {
            seat: address.to_owned(),
            reason: reason.to_owned(),
            at: now_ts(),
        },
    )
    .await;
}

fn outcome_name(outcome: DeliveryOutcome) -> &'static str {
    match outcome {
        DeliveryOutcome::Delivered => "delivered",
        DeliveryOutcome::Deferred => "deferred",
        DeliveryOutcome::Escalated => "escalated",
        DeliveryOutcome::Contaminated => "contaminated",
        DeliveryOutcome::Failed => "failed",
    }
}

fn refusal(session: &str, reason: &str) -> String {
    format!(
        "Refused: '{session}' is at an interactive prompt ({reason}). A message must not select or approve it. To deliberately drive the prompt: rig send {session} \"<text>\" --dangerously-interact --reason \"<why>\". No text was sent."
    )
}

fn reason_text(readiness: &Readiness) -> String {
    match readiness {
        Readiness::Ready => ReadinessCode::Ready.as_str().to_owned(),
        Readiness::Booting => ReadinessCode::Booting.as_str().to_owned(),
        Readiness::Gate(code) => code.as_str().to_owned(),
        Readiness::Inconclusive => ReadinessCode::UnrecognisedScreen.as_str().to_owned(),
        Readiness::HardFailure(text) => text.clone(),
    }
}

fn seat_name(address: &str) -> &str {
    address.split_once('@').map_or(address, |(name, _)| name)
}
