//! Guarded direct send. The nudge path uses the same verdict.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use crewmux_adapters::{Readiness, classify};
use crewmux_core::DomainEvent;
use crewmux_messaging::{
    DeliveryOutcome, DeliveryReceipt, OutboxRecord, PasteDisplay, SessionRef,
    obligation_fingerprint, wrap_reply,
};
use crewmux_mux::{
    CaptureRange, ComposerState, GuardConfig, GuardVerdict, Key, PasteOpts, evaluate,
};
use crewmux_types::{ReadinessCode, RuntimeKind};
use serde_json::{Value, json};

use crate::composer::{ComposerParser, classify_stub, stub_drafting};
use crate::error::Error;
use crate::launch::now_ts;
use crate::notify::emit;
use crate::paste_check::{PasteHit, classify_for, clear_composer, history_range, pasted_screen};
use crate::query::{address_of, live_seats, pane_of};
use crate::state::App;

const REASON_REQUIRED: &str = "--dangerously-interact requires --reason explaining why the prompt is being driven. No text was sent.";

async fn stopped_before_paste(
    app: &App,
    outbound: &Outbound,
    address: &str,
) -> Result<bool, Error> {
    if !outbound.track_direct {
        return Ok(false);
    }
    crate::direct::crash_before_paste(app, address, &outbound.body).await
}

struct PasteRoute {
    bracketed: bool,
    display: PasteDisplay,
    runtime: RuntimeKind,
}

/// How this seat shows a paste, and which reset keys are safe to send.
#[derive(Clone, Copy)]
struct Shown {
    display: PasteDisplay,
    runtime: RuntimeKind,
}

/// One `rig send` or `rig broadcast`.
#[expect(
    clippy::struct_excessive_bools,
    reason = "send flags are independent: broadcast, interact, verify, and direct-row tracking"
)]
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
    /// Record a direct-delivery row. Restore re-sends set this false.
    pub track_direct: bool,
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
    // Check-then-paste matches the TypeScript sender: the process can still
    // change after this returns and before the paste.
    crate::lineage::ensure(app, &seat, &pane).await?;
    let runtime = seat.runtime;
    let shown = Shown {
        display: PasteDisplay::for_runtime(runtime),
        runtime,
    };
    let shot = app.mux.capture(&pane, CaptureRange::Visible).await?;
    let readiness = classify(&shot.text);
    if readiness != Readiness::Ready {
        return not_ready(app, outbound, &address, &pane, &readiness, shown).await;
    }
    guarded(app, outbound, &address, &pane, true, shown).await
}

async fn not_ready(
    app: &App,
    outbound: &Outbound,
    address: &str,
    pane: &crewmux_mux::PaneRef,
    readiness: &Readiness,
    shown: Shown,
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
    if stopped_before_paste(app, outbound, address).await? {
        return Ok(json!({"seat": address, "outcome": "crashed_before_paste"}));
    }
    paste_and_check(
        app,
        outbound,
        address,
        pane,
        reason,
        PasteRoute {
            bracketed: false,
            display: shown.display,
            runtime: shown.runtime,
        },
    )
    .await
}

async fn guarded(
    app: &App,
    outbound: &Outbound,
    address: &str,
    pane: &crewmux_mux::PaneRef,
    bracketed: bool,
    shown: Shown,
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
    if stopped_before_paste(app, outbound, address).await? {
        return Ok(json!({"seat": address, "outcome": "crashed_before_paste"}));
    }
    paste_and_check(
        app,
        outbound,
        address,
        pane,
        "",
        PasteRoute {
            bracketed,
            display: shown.display,
            runtime: shown.runtime,
        },
    )
    .await
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
    interact_reason: &str,
    route: PasteRoute,
) -> Result<Value, Error> {
    // Launch startup, direct send, and restore re-send all paste through here.
    let seat = crate::query::find_address(app, address).await?;
    crate::lineage::ensure(app, &seat, pane).await?;
    let shown = crate::query::reply_target(app, &outbound.from).await;
    let payload = wrap_reply(&shown, address, &outbound.body, &shown);
    // History, joined. The composer region of this capture is the draft at
    // verdict time; rows above the prompt are not part of the comparison.
    let range = history_range(&payload);
    let before = app.mux.capture(pane, range).await?;
    let opts = PasteOpts {
        bracketed: route.bracketed,
        preserve_lf: true,
    };
    tracing::debug!(verify = outbound.verify, seat = address, "paste");
    let (seat_name, rig_name) = split_address(address);
    let cursor = transcript_len(app, rig_name, seat_name).await?;
    app.mux.paste(pane, &payload, opts).await?;
    wait_landed(app, rig_name, seat_name, cursor, &outbound.body).await;
    let after = pasted_screen(app, pane, &before.text, &payload, range, route.display).await?;
    let view = PasteView {
        before: &before.text,
        after: &after,
        payload: &payload,
        range,
        route: &route,
    };
    let (outcome, degraded) = finish_paste(app, address, pane, view).await?;
    stored(
        app,
        outbound,
        address,
        Landed {
            outcome,
            degraded,
            reason: interact_reason,
            cursor,
        },
    )
    .await
}

fn split_address(address: &str) -> (&str, &str) {
    address.split_once('@').unwrap_or((address, ""))
}

async fn transcript_len(app: &App, rig_name: &str, seat_name: &str) -> Result<u64, Error> {
    if app.transcripts {
        crate::transcripts::len_of(&app.home, rig_name, seat_name)
            .await
            .map_err(Error::from)
    } else {
        Ok(0)
    }
}

async fn wait_landed(app: &App, rig_name: &str, seat_name: &str, cursor: u64, body: &str) {
    if app.transcripts {
        crate::transcripts::wait_for_paste(&app.home, rig_name, seat_name, cursor, body).await;
    }
}

struct PasteView<'a> {
    before: &'a str,
    after: &'a str,
    payload: &'a str,
    range: CaptureRange,
    route: &'a PasteRoute,
}

async fn finish_paste(
    app: &App,
    address: &str,
    pane: &crewmux_mux::PaneRef,
    view: PasteView<'_>,
) -> Result<(DeliveryOutcome, bool), Error> {
    let mut degraded = ComposerParser::Stub.state(view.before, None) == ComposerState::Unknown;
    let lines = view.payload.lines().count();
    let outcome = match classify_for(view.route.display, view.before, view.after, view.payload) {
        PasteHit::Clean => {
            app.mux.send_keys(pane, &[Key::Enter]).await?;
            DeliveryOutcome::Delivered
        }
        PasteHit::Contaminated => {
            if !clear_composer(app, pane, view.range, lines, view.route.runtime).await? {
                degraded = true;
            }
            raise(app, address, "contaminated").await;
            DeliveryOutcome::Contaminated
        }
        PasteHit::Missing => {
            if !clear_composer(app, pane, view.range, lines, view.route.runtime).await? {
                degraded = true;
            }
            raise(app, address, "did not render").await;
            DeliveryOutcome::Failed
        }
    };
    Ok((outcome, degraded))
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
        Landed {
            outcome,
            degraded: evaluated.guard_degraded,
            reason: interact_reason.unwrap_or(""),
            cursor: 0,
        },
    )
    .await
}

struct Landed<'a> {
    outcome: DeliveryOutcome,
    degraded: bool,
    reason: &'a str,
    cursor: u64,
}

async fn stored(
    app: &App,
    outbound: &Outbound,
    address: &str,
    landed: Landed<'_>,
) -> Result<Value, Error> {
    let cursor = app.cursor.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let fingerprint = obligation_fingerprint(&outbound.from, address, &outbound.body);
    let receipt = DeliveryReceipt {
        outcome: landed.outcome,
        guard_degraded: landed.degraded,
        fingerprint: fingerprint.clone(),
        cursor,
        at: now_ts(),
        interact_reason: (!landed.reason.is_empty()).then(|| landed.reason.to_owned()),
    };
    let doc = serde_json::to_string(&receipt).map_err(|err| Error::Message(err.to_string()))?;
    let store = Arc::clone(&app.store);
    let inserted = tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::insert_receipt(conn, cursor, &doc))
    })
    .await
    .map_err(crate::state::join_err)?;
    inserted.map_err(|err| Error::Message(format!("receipt: {err}")))?;
    if outbound.track_direct && landed.outcome == DeliveryOutcome::Delivered {
        crate::direct::note_pasted(app, address, &outbound.body, landed.cursor).await?;
    }
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
            outcome: outcome_name(landed.outcome).to_owned(),
            at: now_ts(),
        },
    )
    .await;
    Ok(json!({
        "seat": address,
        "outcome": outcome_name(landed.outcome),
        "guard_degraded": landed.degraded,
        "fingerprint": fingerprint,
        "cursor": cursor,
        "bracketed": landed.reason.is_empty(),
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
        Readiness::Login { evidence, .. } => evidence.clone(),
    }
}

fn seat_name(address: &str) -> &str {
    address.split_once('@').map_or(address, |(name, _)| name)
}
