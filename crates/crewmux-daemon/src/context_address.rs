// Derived from openrig packages/daemon/src/routes/context-packs.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `GET /library/resolve-address`. Longest-prefix pack match, then the file span.

use crewmux_orchestration::context_packs::Library;
use crewmux_spec::context::{AddressError, parse_address, resolve_address};
use serde_json::{Value, json};

use crate::context_http::RouteError;

pub(crate) fn resolve_address_route(
    lib: &mut Library,
    address: Option<&str>,
) -> Result<Value, RouteError> {
    let Some(address) = address.filter(|text| !text.is_empty()) else {
        return Err(RouteError::named(
            400,
            "missing_address",
            "address is required: <pack-ref>/<file>[#H2-slug[/H3-slug]]",
        ));
    };
    let parsed = parse_address(address)
        .map_err(|err: AddressError| RouteError::named(400, "invalid_address", err.message))?;
    let segments: Vec<&str> = parsed
        .refer
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let entries = lib.list();
    let mut matched = None;
    for cut in (1..segments.len()).rev() {
        let candidate = segments
            .iter()
            .take(cut)
            .copied()
            .collect::<Vec<_>>()
            .join("/");
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.relative_path == candidate)
        {
            let file_path = segments
                .iter()
                .skip(cut)
                .copied()
                .collect::<Vec<_>>()
                .join("/");
            matched = Some(((*entry).clone(), file_path));
            break;
        }
    }
    let Some((entry, file_path)) = matched else {
        return Err(RouteError::named(
            404,
            "pack_not_found",
            format!(
                "no library pack matches any prefix of '{}' — run 'rig context list' for the available refs",
                parsed.refer
            ),
        ));
    };
    if file_path.is_empty() {
        return Err(RouteError::named(
            400,
            "missing_file_path",
            format!(
                "'{}' names pack '{}' but no file within it — the addressable unit is the file: <pack-ref>/<file>[#...]",
                parsed.refer, entry.relative_path
            ),
        ));
    }
    file_body(address, &parsed, &entry, &file_path)
}

fn file_body(
    address: &str,
    parsed: &crewmux_spec::context::ParsedAddress,
    entry: &crewmux_orchestration::context_packs::PackEntry,
    file_path: &str,
) -> Result<Value, RouteError> {
    if !entry.files.iter().any(|file| file.path == file_path) {
        let names = entry
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(RouteError::named(
            404,
            "file_not_in_pack",
            format!(
                "pack '{}' declares no file '{file_path}' — declared: {names}",
                entry.relative_path
            ),
        ));
    }
    let file_text = Library::read_pack_text(entry, file_path).map_err(|err| {
        RouteError::named(
            422,
            "file_unreadable",
            format!(
                "pack '{}' file '{file_path}': {}",
                entry.relative_path, err.message
            ),
        )
    })?;
    if parsed.header_path.is_empty() {
        return Ok(json!({
            "address": address,
            "packRef": entry.relative_path,
            "filePath": file_path,
            "text": file_text,
        }));
    }
    match resolve_address(&file_text, &parsed.header_path) {
        Ok(section) => Ok(json!({
            "address": address,
            "packRef": entry.relative_path,
            "filePath": file_path,
            "headerPath": section.header_path,
            "headerLine": section.header_line,
            "text": section.text,
            "ownText": section.own_text,
        })),
        Err(err) => Err(RouteError::named(
            422,
            "address_unresolved",
            format!("{}: {}", parsed.refer, err.message),
        )),
    }
}
