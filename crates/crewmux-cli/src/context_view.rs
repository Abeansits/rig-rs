// Derived from openrig packages/cli/src/commands/context.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Daemon-backed `rig context` verbs.

use std::path::Path;

use crewmux_api::{
    ContextByRef, ContextCompose, ContextDeliver, ContextLibrary, ContextPieces, ContextPreview,
    ContextProfile, ContextRecap, ContextSync, Endpoint,
};
use serde_json::{Value, json};

use crate::context_call::{call, escape, expect, fail, field, push_query};
use crate::error::Error;
use crate::output;

pub(crate) struct ProfileArgs<'a> {
    pub name_or_ref: &'a str,
    pub situation: &'a str,
    pub runtime: Option<&'a str>,
    pub profile: Option<&'a str>,
    pub budget: Option<&'a str>,
    pub rig: Option<&'a str>,
    pub seat: Option<&'a str>,
    pub mission: Option<&'a str>,
    pub slice: Option<&'a str>,
    pub json: bool,
}

pub(crate) fn list(json_out: bool) -> Result<i32, Error> {
    let (status, value) = call(ContextLibrary::METHOD, ContextLibrary::PATH, None)?;
    expect(status, 200, &value)?;
    if json_out {
        output::stdout_json(&value);
        return Ok(0);
    }
    let entries = value.as_array().cloned().unwrap_or_default();
    if entries.is_empty() {
        output::stdout_line(
            "No context packs in library. Author one under the context root, then run: rig context sync",
        );
        return Ok(0);
    }
    for entry in entries {
        output::stdout_line(&format!(
            "{}  {}  v{}  {}  {} files  {}",
            field(&entry, "relativePath"),
            field(&entry, "name"),
            field(&entry, "version"),
            field(&entry, "taxonomy"),
            entry
                .get("files")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
            field(&entry, "sourceType"),
        ));
    }
    Ok(0)
}

pub(crate) fn show(name: &str, json_out: bool) -> Result<i32, Error> {
    let entry = resolve(name)?;
    if json_out {
        output::stdout_json(&entry);
    } else {
        output::stdout_line(&format!("Ref:     {}", field(&entry, "relativePath")));
        output::stdout_line(&format!("Name:    {}", field(&entry, "name")));
        output::stdout_line(&format!("Version: {}", field(&entry, "version")));
        output::stdout_line(&format!(
            "Source:  {} ({})",
            field(&entry, "sourceType"),
            field(&entry, "sourcePath")
        ));
    }
    Ok(0)
}

pub(crate) fn preview(name: &str, json_out: bool) -> Result<i32, Error> {
    let entry = resolve(name)?;
    let refer = field(&entry, "relativePath");
    let (status, value) = call(
        ContextPreview::METHOD,
        &format!("{}?ref={}", ContextPreview::PATH, escape(&refer)),
        None,
    )?;
    expect(status, 200, &value)?;
    if json_out {
        output::stdout_json(&value);
        return Ok(0);
    }
    output::stdout_line(&format!(
        "# Preview: {} v{}",
        field(&value, "name"),
        field(&value, "version")
    ));
    output::stdout_line(field(&value, "bundleText").trim_end());
    Ok(0)
}

pub(crate) fn get(name: &str, json_out: bool) -> Result<i32, Error> {
    if name.contains('#') {
        return address(name, json_out);
    }
    if name.contains('/') {
        let (status, value) = call(
            ContextByRef::METHOD,
            &format!("{}?ref={}", ContextByRef::PATH, escape(name)),
            None,
        )?;
        if status == 200 {
            return preview_bytes(&value, json_out);
        }
        if status == 404 {
            return address(name, json_out);
        }
        return fail(status, &value);
    }
    preview(name, json_out)
}

pub(crate) fn compose(out_ref: &str, from: &[String]) -> Result<i32, Error> {
    let sources: Vec<Value> = from
        .iter()
        .map(|path| {
            json!({
                "path": std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf()).display().to_string(),
                "label": path,
            })
        })
        .collect();
    let body = serde_json::to_vec(&json!({"outRef": out_ref, "sources": sources}))
        .map_err(|err| Error::Json(err.to_string()))?;
    let (status, value) = call(ContextCompose::METHOD, ContextCompose::PATH, Some(&body))?;
    expect(status, 201, &value)?;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .map_or(from.len(), Vec::len);
    output::stdout_line(&format!(
        "Composed {files} file(s) -> {} ({} bytes, ~{} tokens).",
        field(&value, "ref"),
        value.get("bytes").and_then(Value::as_u64).unwrap_or(0),
        value
            .get("estimatedTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    ));
    Ok(0)
}

pub(crate) fn sync(json_out: bool) -> Result<i32, Error> {
    let (status, value) = call(ContextSync::METHOD, ContextSync::PATH, Some(b"{}"))?;
    expect(status, 200, &value)?;
    if json_out {
        output::stdout_json(&value);
    } else {
        output::stdout_line(&format!(
            "Indexed {} context pack(s).",
            value.get("count").and_then(Value::as_u64).unwrap_or(0)
        ));
    }
    Ok(0)
}

pub(crate) fn sync_count() -> Result<u64, Error> {
    let (status, value) = call(ContextSync::METHOD, ContextSync::PATH, Some(b"{}"))?;
    expect(status, 200, &value)?;
    Ok(value.get("count").and_then(Value::as_u64).unwrap_or(0))
}

pub(crate) fn remove(refer: &str, json_out: bool) -> Result<i32, Error> {
    let (status, value) = call(
        "DELETE",
        &format!("{}?ref={}", ContextByRef::PATH, escape(refer)),
        None,
    )?;
    expect(status, 200, &value)?;
    if json_out {
        output::stdout_json(&value);
    } else {
        output::stdout_line(&format!(
            "Removed context pack '{}'. Library now has {} context pack(s).",
            field(&value, "ref"),
            value.get("count").and_then(Value::as_u64).unwrap_or(0)
        ));
    }
    Ok(0)
}

pub(crate) fn deliver(refer: &str, seat: &str, mode: &str, json_out: bool) -> Result<i32, Error> {
    let body = serde_json::to_vec(&json!({"ref": refer, "seat": seat, "mode": mode}))
        .map_err(|err| Error::Json(err.to_string()))?;
    let (status, value) = call(ContextDeliver::METHOD, ContextDeliver::PATH, Some(&body))?;
    expect(status, 200, &value)?;
    if json_out {
        output::stdout_json(&value);
    } else {
        output::stdout_line(&format!(
            "Delivered {} to {seat} via {mode}.",
            field(&value, "ref")
        ));
    }
    Ok(0)
}

pub(crate) fn recap_write(rig: &str, seat: &str, file: &Path) -> Result<i32, Error> {
    let content = std::fs::read_to_string(file)
        .map_err(|err| Error::User(format!("failed to read {}: {err}", file.display())))?;
    let body = serde_json::to_vec(&json!({"rig": rig, "seat": seat, "content": content}))
        .map_err(|err| Error::Json(err.to_string()))?;
    let (status, value) = call(ContextRecap::METHOD, ContextRecap::PATH, Some(&body))?;
    expect(status, 200, &value)?;
    output::stdout_line(&format!("Wrote {}", field(&value, "path")));
    if let Some(findings) = value.get("findings").and_then(Value::as_array) {
        for finding in findings {
            output::stderr_line(&finding.to_string());
        }
    }
    Ok(0)
}

pub(crate) fn profile(args: &ProfileArgs<'_>) -> Result<i32, Error> {
    let entry = resolve(args.name_or_ref)?;
    let refer = field(&entry, "relativePath");
    let runtime = match args.runtime {
        Some("claude-code" | "claude") | None => "claude",
        Some("codex") => "codex",
        Some(other) => {
            return Err(Error::User(format!(
                "invalid_runtime: --runtime must be claude-code (alias: claude) or codex (got: {other})"
            )));
        }
    };
    let mut path = format!(
        "{}?ref={}&situation={}&runtime={runtime}",
        ContextProfile::PATH,
        escape(&refer),
        escape(args.situation)
    );
    push_query(&mut path, "profile", args.profile);
    push_query(&mut path, "budget", args.budget);
    push_query(&mut path, "rig", args.rig);
    push_query(&mut path, "seat", args.seat);
    push_query(&mut path, "mission", args.mission);
    push_query(&mut path, "slice", args.slice);
    let (status, value) = call(ContextProfile::METHOD, &path, None)?;
    expect(status, 200, &value)?;
    if args.json {
        output::stdout_json(&value);
    } else {
        let count = value
            .get("pieces")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        output::stdout_line(&format!(
            "{} {} {runtime}: {count} pieces, ~{} tokens",
            field(&value, "ref"),
            field(&value, "situation"),
            value
                .get("totalEstimatedTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ));
    }
    Ok(0)
}

fn preview_bytes(entry: &Value, json_out: bool) -> Result<i32, Error> {
    let refer = field(entry, "relativePath");
    let (status, value) = call(
        ContextPieces::METHOD,
        &format!("{}?ref={}", ContextPieces::PATH, escape(&refer)),
        None,
    )?;
    expect(status, 200, &value)?;
    if json_out {
        output::stdout_json(&value);
    } else {
        output::stdout_line(&field(&value, "text"));
    }
    Ok(0)
}

fn address(name: &str, json_out: bool) -> Result<i32, Error> {
    let (status, value) = call(
        "GET",
        &format!(
            "/api/context-packs/library/resolve-address?address={}",
            escape(name)
        ),
        None,
    )?;
    expect(status, 200, &value)?;
    if json_out {
        output::stdout_json(&value);
    } else {
        output::stdout_line(&field(&value, "text"));
    }
    Ok(0)
}

fn resolve(name: &str) -> Result<Value, Error> {
    if name.starts_with("context-pack:") {
        return Err(Error::User(
            "Context pack colon-id addressing ('context-pack:<name>:<version>') was removed. Address a pack by its path-like ref instead (for example 'packs/compaction-restore')."
                .to_owned(),
        ));
    }
    if name.contains('/') {
        let (status, value) = call(
            ContextByRef::METHOD,
            &format!("{}?ref={}", ContextByRef::PATH, escape(name)),
            None,
        )?;
        if status == 200 {
            return Ok(value);
        }
        return fail(status, &value);
    }
    let (status, value) = call(ContextLibrary::METHOD, ContextLibrary::PATH, None)?;
    expect(status, 200, &value)?;
    let entries = value.as_array().cloned().unwrap_or_default();
    if let Some(exact) = entries
        .iter()
        .find(|entry| field(entry, "relativePath") == name)
    {
        return Ok(exact.clone());
    }
    let matches: Vec<Value> = entries
        .into_iter()
        .filter(|entry| field(entry, "name") == name)
        .collect();
    match matches.len() {
        1 => matches
            .first()
            .cloned()
            .ok_or_else(|| Error::User("missing pack".to_owned())),
        0 => Err(Error::User(format!(
            "Context pack '{name}' not found in library. Run 'rig context list' to see what's available."
        ))),
        _ => Err(Error::User(format!(
            "Context pack name '{name}' is ambiguous. Address it by path-like ref."
        ))),
    }
}
