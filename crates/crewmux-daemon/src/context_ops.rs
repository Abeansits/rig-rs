// Derived from openrig packages/daemon/src/routes/context-packs.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Library list, sync, compose, show, remove, preview, and pieces.

use std::path::PathBuf;

use crewmux_orchestration::context_packs::{
    ComposeFile, Library, PackCode, PackError, PlainInput, assemble_bundle, assemble_plain,
};
use serde_json::{Value, json};

use crate::context_http::RouteError;

#[expect(
    clippy::unnecessary_wraps,
    reason = "the blocking route runner requires Result"
)]
pub(crate) fn library_list(lib: &mut Library) -> Result<Value, RouteError> {
    Ok(entries_json(lib))
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "the blocking route runner requires Result"
)]
pub(crate) fn sync_route(lib: &mut Library) -> Result<Value, RouteError> {
    let report = lib.scan();
    let errors: Vec<Value> = report
        .errors
        .iter()
        .map(|err| json!({"source": err.source, "error": err.error}))
        .collect();
    Ok(json!({
        "count": report.count,
        "errors": errors,
        "entries": entries_json(lib),
    }))
}

pub(crate) fn compose_route(lib: &mut Library, text: &str) -> Result<Value, RouteError> {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let Some(out_ref) = value.get("outRef").and_then(Value::as_str) else {
        return Err(bad_compose());
    };
    let Some(sources) = value.get("sources").and_then(Value::as_array) else {
        return Err(bad_compose());
    };
    let mut files = Vec::new();
    for source in sources {
        let Some(path) = source.get("path").and_then(Value::as_str) else {
            return Err(bad_compose());
        };
        let Some(label) = source.get("label").and_then(Value::as_str) else {
            return Err(bad_compose());
        };
        files.push(ComposeFile {
            path: PathBuf::from(path),
            label: label.to_owned(),
        });
    }
    let outcome = lib.compose_from_files(out_ref, &files)?;
    let members: Vec<Value> = outcome
        .files
        .iter()
        .map(|file| {
            json!({
                "path": file.path,
                "bytes": file.bytes,
                "estimatedTokens": file.estimated_tokens,
            })
        })
        .collect();
    Ok(json!({
        "ref": outcome.refer,
        "entry": outcome.entry.to_json(),
        "text": outcome.text,
        "bytes": outcome.bytes,
        "estimatedTokens": outcome.estimated_tokens,
        "files": members,
        "missingFiles": [],
    }))
}

pub(crate) fn show_route(lib: &mut Library, refer: Option<&str>) -> Result<Value, RouteError> {
    let entry = require_entry(lib, refer)?;
    Ok(entry.to_json())
}

pub(crate) fn remove_route(lib: &mut Library, refer: Option<&str>) -> Result<Value, RouteError> {
    let refer = require_query(refer)?;
    let outcome = lib.remove_by_ref(refer)?;
    Ok(json!({
        "removed": outcome.removed,
        "ref": outcome.refer,
        "removedPath": outcome.removed_path.display().to_string(),
        "count": lib.list().len(),
    }))
}

pub(crate) fn preview_route(lib: &mut Library, refer: Option<&str>) -> Result<Value, RouteError> {
    let entry = require_entry(lib, refer)?;
    let bundle = assemble_bundle(&entry)?;
    let files: Vec<Value> = bundle
        .files
        .iter()
        .map(|file| {
            json!({
                "path": file.path,
                "role": file.role,
                "bytes": file.bytes,
                "estimatedTokens": file.estimated_tokens,
            })
        })
        .collect();
    let missing: Vec<Value> = bundle
        .missing
        .iter()
        .map(|(path, role)| json!({"path": path, "role": role}))
        .collect();
    Ok(json!({
        "id": entry.id,
        "name": entry.name,
        "version": entry.version,
        "bundleText": bundle.text,
        "bundleBytes": bundle.bytes,
        "estimatedTokens": bundle.estimated_tokens,
        "files": files,
        "missingFiles": missing,
    }))
}

pub(crate) fn pieces_route(lib: &mut Library, refer: Option<&str>) -> Result<Value, RouteError> {
    let entry = require_entry(lib, refer)?;
    let mut pieces = Vec::new();
    let mut missing = Vec::new();
    let mut owned = Vec::new();
    for file in &entry.files {
        if file.absolute_path.is_none() {
            missing.push(json!({"path": file.path, "role": file.role}));
            owned.push((file.path.clone(), None));
            continue;
        }
        if let Ok(content) = read_member(&entry, &file.path) {
            pieces.push(json!({"path": file.path, "role": file.role, "content": &content}));
            owned.push((file.path.clone(), Some(content)));
        } else {
            missing.push(json!({"path": file.path, "role": file.role}));
            owned.push((file.path.clone(), None));
        }
    }
    let inputs: Vec<PlainInput<'_>> = owned
        .iter()
        .map(|(path, content)| PlainInput {
            path,
            content: content.as_deref(),
        })
        .collect();
    let assembled = assemble_plain(&inputs);
    Ok(json!({
        "ref": entry.relative_path,
        "id": entry.id,
        "pieces": pieces,
        "missingFiles": missing,
        "text": assembled.text,
        "bytes": assembled.bytes,
    }))
}

fn read_member(
    entry: &crewmux_orchestration::context_packs::PackEntry,
    rel: &str,
) -> Result<String, PackError> {
    let path = Library::resolve_file(entry, rel)?;
    std::fs::read_to_string(&path).map_err(|err| {
        PackError::new(
            PackCode::FileReadFailed,
            format!("failed to read {}: {err}", path.display()),
        )
    })
}

fn entries_json(lib: &Library) -> Value {
    Value::Array(lib.list().iter().map(|entry| entry.to_json()).collect())
}

fn require_query(refer: Option<&str>) -> Result<&str, RouteError> {
    match refer {
        Some(refer) if !refer.is_empty() => Ok(refer),
        _ => Err(RouteError::named(
            400,
            "ref_required",
            "query must include ?ref=<path-like-ref>",
        )),
    }
}

fn require_entry(
    lib: &Library,
    refer: Option<&str>,
) -> Result<crewmux_orchestration::context_packs::PackEntry, RouteError> {
    let refer = require_query(refer)?;
    match lib.by_ref(refer) {
        Ok(Some(entry)) => Ok(entry.clone()),
        Ok(None) => Err(RouteError::named(
            404,
            "pack_not_found",
            format!("Context pack '{refer}' not found in library"),
        )),
        Err(err) => Err(RouteError::Pack(err)),
    }
}

fn bad_compose() -> RouteError {
    RouteError::named(
        400,
        "invalid_compose_request",
        "body must include { outRef, sources: [{ path, label }, ...] }",
    )
}
