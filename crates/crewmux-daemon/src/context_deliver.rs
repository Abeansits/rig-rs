//! Paste a composed pack through the slice 4 guard, or hand it to `deliver_startup`.

use std::path::PathBuf;

use async_trait::async_trait;
use crewmux_adapters::{AgentAdapter, ResolvedStartupFile, SeatIo, SubmitMode};
use crewmux_messaging::ContextPaste;
use crewmux_mux::Capture;
use crewmux_mux::PaneRef;
use crewmux_orchestration::context_packs::{
    Library, PackCode, PackError, PlainInput, assemble_plain,
};
use serde_json::{Value, json};

use crate::context_http::RouteError;
use crate::deliver::{Outbound, deliver};
use crate::state::App;

pub(crate) struct Prepared {
    refer: String,
    mode: String,
    seat: String,
    paste: ContextPaste,
    path: String,
}

pub(crate) fn prepare(lib: &mut Library, body: &[u8]) -> Result<Prepared, RouteError> {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let refer = value
        .get("ref")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty());
    let seat = value
        .get("seat")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty());
    let mode = value.get("mode").and_then(Value::as_str);
    let (Some(refer), Some(seat), Some(mode)) = (refer, seat, mode) else {
        return Err(RouteError::named(
            400,
            "invalid_deliver_request",
            "body must include { ref, seat, mode: paste|startup }",
        ));
    };
    if mode != "paste" && mode != "startup" {
        return Err(RouteError::named(
            400,
            "invalid_deliver_mode",
            format!("mode must be paste | startup (got: {mode})"),
        ));
    }
    let entry = match lib.by_ref(refer) {
        Ok(Some(entry)) => entry.clone(),
        Ok(None) => {
            return Err(RouteError::named(
                404,
                "pack_not_found",
                format!("Context pack '{refer}' not found in library"),
            ));
        }
        Err(err) => return Err(RouteError::Pack(err)),
    };
    let mut owned = Vec::new();
    for file in &entry.files {
        let Some(path) = &file.absolute_path else {
            return Err(missing(&file.path));
        };
        let text = std::fs::read_to_string(path).map_err(|err| {
            RouteError::Pack(PackError::new(
                PackCode::FileReadFailed,
                format!("failed to read {}: {err}", path.display()),
            ))
        })?;
        owned.push((file.path.clone(), text));
    }
    if owned.is_empty() {
        return Err(missing("(none)"));
    }
    let inputs: Vec<PlainInput<'_>> = owned
        .iter()
        .map(|(path, content)| PlainInput {
            path,
            content: Some(content.as_str()),
        })
        .collect();
    let assembled = assemble_plain(&inputs);
    Ok(Prepared {
        refer: refer.to_owned(),
        mode: mode.to_owned(),
        seat: seat.to_owned(),
        path: "context-pack.md".to_owned(),
        paste: ContextPaste {
            body: assembled.text,
            tokens_est: assembled.estimated_tokens,
            sources: vec![refer.to_owned()],
        },
    })
}

pub(crate) async fn deliver_pack(app: &App, prepared: Prepared) -> Result<Value, RouteError> {
    if prepared.mode == "startup" {
        return startup(app, &prepared).await;
    }
    let seat = prepared.seat.clone();
    let tokens = prepared.paste.tokens_est;
    let refer = prepared.refer.clone();
    let delivery = deliver(
        app,
        Outbound {
            from: "context".to_owned(),
            targets: vec![prepared.seat],
            rig: None,
            all: false,
            body: prepared.paste.body,
            interact: false,
            reason: None,
            verify: true,
            track_direct: true,
        },
    )
    .await
    .map_err(|err| RouteError::named(400, "deliver_failed", err.to_string()))?;
    Ok(json!({
        "mode": "paste",
        "ref": refer,
        "seat": seat,
        "tokensEst": tokens,
        "delivery": delivery,
    }))
}

async fn startup(app: &App, prepared: &Prepared) -> Result<Value, RouteError> {
    let seat = crate::query::find_address(app, &prepared.seat)
        .await
        .map_err(|err| RouteError::named(404, "seat_not_found", err.to_string()))?;
    let adapter = app.registry.get(seat.runtime).ok_or_else(|| {
        RouteError::named(
            400,
            "runtime_unavailable",
            format!("no adapter for {}", seat.runtime.as_str()),
        )
    })?;
    let session = seat.tmux_session.clone().ok_or_else(|| {
        RouteError::named(
            400,
            "seat_not_launched",
            format!("seat {} has no session", seat.name.as_str()),
        )
    })?;
    let cwd = seat.cwd.clone().unwrap_or_else(PathBuf::new);
    let mut binding = crewmux_adapters::binding_for(seat.id, cwd, &session);
    binding.pane = PaneRef {
        session,
        window: Some(0),
        pane: Some(0),
        pane_id: seat.pane_id.clone(),
    };
    let files = [ResolvedStartupFile {
        path: prepared.path.clone(),
        body: prepared.paste.body.clone(),
    }];
    let io = NoPaste;
    let result = AgentAdapter::deliver_startup(adapter.as_ref(), &files, &binding, &io)
        .await
        .map_err(|err| RouteError::named(500, "startup_failed", err.to_string()))?;
    Ok(json!({
        "mode": "startup",
        "ref": prepared.refer,
        "seat": prepared.seat,
        "path": prepared.path,
        "delivered": result.delivered,
        "tokensEst": prepared.paste.tokens_est,
    }))
}

fn missing(path: &str) -> RouteError {
    RouteError::named(
        400,
        "missing_files",
        format!("context pack delivery requires every declared file; missing {path}"),
    )
}

/// Startup delivery must not submit a turn or send Enter.
struct NoPaste;

#[async_trait]
impl SeatIo for NoPaste {
    async fn submit(
        &self,
        _text: &str,
        _mode: SubmitMode,
    ) -> Result<crewmux_adapters::DeliveryReceipt, crewmux_adapters::Error> {
        Err(crewmux_adapters::Error::Message(
            "startup delivery does not submit a turn".to_owned(),
        ))
    }

    async fn interrupt(&self) -> Result<(), crewmux_adapters::Error> {
        Ok(())
    }

    async fn snapshot(&self) -> Result<Capture, crewmux_adapters::Error> {
        Ok(Capture::new(String::new(), (0, 0)))
    }
}
