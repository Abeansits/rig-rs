//! Paste a composed pack through the slice 4 guard.
//!
//! `mode: startup` is `not_implemented`. The stub adapter's `deliver_startup`
//! counts files and writes nothing, so this route does not call it.

use crewmux_messaging::ContextPaste;
use crewmux_orchestration::context_packs::{Library, PlainInput, assemble_plain};
use serde_json::{Value, json};

use crate::context_http::RouteError;
use crate::deliver::{Outbound, deliver};
use crate::state::App;

pub(crate) struct Prepared {
    refer: String,
    mode: String,
    seat: String,
    paste: ContextPaste,
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
    if mode == "startup" {
        return Err(startup_unimplemented());
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
        if file.absolute_path.is_none() {
            return Err(missing(&file.path));
        }
        let text = Library::read_pack_text(&entry, &file.path)?;
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
        paste: ContextPaste {
            body: assembled.text,
            tokens_est: assembled.estimated_tokens,
            sources: vec![refer.to_owned()],
        },
    })
}

pub(crate) async fn deliver_pack(app: &App, prepared: Prepared) -> Result<Value, RouteError> {
    if prepared.mode == "startup" {
        return Err(startup_unimplemented());
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

fn startup_unimplemented() -> RouteError {
    RouteError::named(
        501,
        "not_implemented",
        "context pack startup delivery is not implemented; use mode paste",
    )
}

fn missing(path: &str) -> RouteError {
    RouteError::named(
        400,
        "missing_files",
        format!("context pack delivery requires every declared file; missing {path}"),
    )
}
