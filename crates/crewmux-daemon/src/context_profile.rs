// Derived from openrig packages/daemon/src/domain/context-packs/profile-composer.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `GET /library/by-ref/profile`. Situation algebra over the pack's atoms.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crewmux_orchestration::context_packs::{
    ComposedPiece, ComposedProfile, ContextAtoms, Library, NamedCompose, SourceRead, SourceRoots,
    compose_named_profile, compose_profile, read_source,
};
use crewmux_spec::context::{
    Atom, AtomRuntime, PackManifest, Priority, Profile, ProfileRuntime, ProfileSource, Purpose,
    Situation, Taxonomy, is_safe_segment, parse_address, parse_manifest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::context_http::{QueryView, RouteError};

pub(crate) fn profile_route(
    lib: &mut Library,
    query: &QueryView,
    grants: &(PathBuf, PathBuf, PathBuf),
) -> Result<Value, RouteError> {
    let entry = crate::context_ops::show_route(lib, query.refer.as_deref())?;
    let refer = entry
        .get("relativePath")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let source = entry
        .get("sourcePath")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let situation = situation_of(query.situation.as_deref())?;
    let runtime = runtime_of(query.runtime.as_deref())?;
    let budget = budget_of(query.budget.as_deref())?;
    let manifest_path = PathBuf::from(&source).join("manifest.yaml");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|err| RouteError::named(422, "manifest_unreadable", err.to_string()))?;
    let manifest = parse_manifest(&raw, &manifest_path.display().to_string())
        .map_err(|err| RouteError::named(422, "manifest_unreadable", err.message))?;
    if manifest.atoms.is_empty() {
        return Err(RouteError::named(
            422,
            "no_atoms",
            format!(
                "pack '{refer}' declares no atoms — a profile composes from atom metadata (mini-req 1); add an atoms: section to its manifest"
            ),
        ));
    }
    let selected = select_profile(
        &manifest,
        query.profile.as_deref(),
        &refer,
        situation,
        runtime,
    )?;
    check_grant(query, selected)?;
    let (topology, slices, workspace) = grants;
    let mut roots = SourceRoots::default();
    let mut atoms = manifest.atoms.clone();
    let mut context = ContextAtoms::default();
    let mut meta = BTreeMap::new();
    let mut warnings = Vec::new();
    if query.rig.is_some() || query.seat.is_some() {
        let (rig, seat) = seat_pair(query)?;
        roots.seat = Some(topology.join("rigs").join(&rig).join("seats").join(&seat));
        context.seat.push(learned_atom());
        meta.insert("profile-seat-learned".to_owned(), ("seat", "default"));
    }
    if let Some(mission) = query.mission.as_deref() {
        crate::context_legacy::apply(
            &mut atoms,
            &mut context,
            &mut meta,
            &mut warnings,
            &mut roots,
            &crate::context_legacy::LegacyQuery {
                refer: &refer,
                situation,
                profile: selected,
                mission,
                slice: query.slice.as_deref(),
                slices,
                workspace,
            },
        )?;
    }
    compose_notes(Job {
        query,
        refer: &refer,
        source: &source,
        atoms: &atoms,
        selected,
        context: &context,
        situation,
        runtime,
        budget,
        roots: &roots,
        meta: &meta,
        warnings,
    })
}

struct Job<'a> {
    query: &'a QueryView,
    refer: &'a str,
    source: &'a str,
    atoms: &'a [Atom],
    selected: Option<&'a crewmux_spec::context::Profile>,
    context: &'a ContextAtoms,
    situation: Situation,
    runtime: ProfileRuntime,
    budget: Option<u32>,
    roots: &'a SourceRoots,
    meta: &'a BTreeMap<String, (&'a str, &'a str)>,
    warnings: Vec<String>,
}

fn compose_notes(mut job: Job<'_>) -> Result<Value, RouteError> {
    let mut notes = BTreeMap::new();
    let pack_dir = PathBuf::from(job.source);
    let mut read = |address: &str| match read_source(&pack_dir, job.roots, address) {
        Ok((text, record)) => {
            notes.insert(address.to_owned(), note(&record));
            Ok(text)
        }
        Err(err) => Err(err),
    };
    let composed = if let Some(profile) = job.selected {
        compose_named_profile(
            &NamedCompose {
                atoms: job.atoms,
                profile,
                situation: job.situation,
                runtime: job.runtime,
                context: job.context,
                budget_tokens: job.budget,
            },
            &mut read,
        )
    } else {
        compose_profile(job.atoms, job.situation, job.runtime, job.budget, &mut read)
    }
    .map_err(|err| RouteError::named(422, "profile_compose_failed", err.message))?;
    if !composed.skipped.is_empty() {
        let rig = job.query.rig.as_deref().unwrap_or("");
        let seat = job.query.seat.as_deref().unwrap_or("");
        job.warnings.push(format!(
            "no seat recap for {seat}@{rig}; read your newest restore map"
        ));
    }
    Ok(project_json(
        job.refer,
        &composed,
        &notes,
        job.meta,
        &job.warnings,
    ))
}

struct Note {
    nominal: String,
    real: String,
    escapes: bool,
    written_at: Option<String>,
}

fn note(record: &SourceRead) -> Note {
    Note {
        nominal: record.nominal_path.display().to_string(),
        real: record.real_path.display().to_string(),
        escapes: record.escapes_root,
        written_at: record.written_at.clone(),
    }
}

fn project_json(
    refer: &str,
    profile: &ComposedProfile,
    notes: &BTreeMap<String, Note>,
    meta: &BTreeMap<String, (&str, &str)>,
    warnings: &[String],
) -> Value {
    let pieces: Vec<Value> = profile
        .pieces
        .iter()
        .map(|piece| piece_json(piece, notes, meta))
        .collect();
    let provenance: Vec<Value> = pieces
        .iter()
        .filter(|piece| piece.pointer("/provenance/escapesRoot").is_some_and(|v| v.as_bool() == Some(true)))
        .map(|piece| {
            let atom = piece.get("atomId").and_then(Value::as_str).unwrap_or("");
            let address = piece.get("address").and_then(Value::as_str).unwrap_or("");
            let kind = piece.get("sourceKind").and_then(Value::as_str).unwrap_or("");
            let real = piece
                .pointer("/provenance/realPath")
                .and_then(Value::as_str)
                .unwrap_or("");
            json!(format!(
                "piece '{atom}' ({address}): bytes came from OUTSIDE its {kind} root — real path {real}"
            ))
        })
        .collect();
    let mut body = json!({
        "ref": refer,
        "situation": profile.situation.as_str(),
        "runtime": profile.runtime.as_str(),
        "pieces": pieces,
        "totalEstimatedTokens": profile.total_estimated_tokens,
        "warnings": warnings,
        "provenanceWarnings": provenance,
    });
    if let Some(id) = &profile.profile_id {
        put(&mut body, "profileId", json!(id));
    }
    if profile.profile_id.is_some() {
        let phases = phase_json(profile, &body);
        put(&mut body, "phases", phases);
    }
    if !profile.skipped.is_empty() {
        put(
            &mut body,
            "skipped",
            json!(
                profile
                    .skipped
                    .iter()
                    .map(|(id, address)| json!({"atomId": id, "address": address}))
                    .collect::<Vec<_>>()
            ),
        );
    }
    if let Some(budget) = &profile.budget {
        put(
            &mut body,
            "budget",
            json!({
                "limitTokens": budget.limit_tokens,
                "overageTokens": budget.overage_tokens,
                "dropCandidates": budget.drop_candidates.iter().map(|item| json!({
                    "atomId": item.atom_id,
                    "priority": item.priority.as_str(),
                    "estimatedTokens": item.estimated_tokens,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    body
}

fn phase_json(profile: &ComposedProfile, body: &Value) -> Value {
    let phases: Vec<Value> = profile
        .phases
        .iter()
        .map(|phase| {
            let phase_pieces: Vec<Value> = body
                .get("pieces")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|piece| {
                    piece.get("phaseId").and_then(Value::as_str) == Some(phase.id.as_str())
                })
                .cloned()
                .collect();
            json!({
                "id": phase.id,
                "kind": phase.kind,
                "sources": phase.sources.iter().map(|source| source.as_str()).collect::<Vec<_>>(),
                "pieces": phase_pieces,
                "estimatedTokens": phase.estimated_tokens,
            })
        })
        .collect();
    json!(phases)
}

fn put(body: &mut Value, key: &str, value: Value) {
    if let Some(object) = body.as_object_mut() {
        object.insert(key.to_owned(), value);
    }
}

fn piece_json(
    piece: &ComposedPiece,
    notes: &BTreeMap<String, Note>,
    meta: &BTreeMap<String, (&str, &str)>,
) -> Value {
    let refer = match parse_address(&piece.address) {
        Ok(parsed) => parsed.refer,
        Err(_) => piece.address.clone(),
    };
    let mut body = json!({
        "atomId": piece.atom_id,
        "address": piece.address,
        "sourceKind": piece.source_kind,
        "order": piece.order,
        "priority": piece.priority.as_str(),
        "text": piece.text,
        "estimatedTokens": piece.estimated_tokens,
        "sha256": sha256_hex(&piece.text),
    });
    if let Some(phase) = &piece.phase_id {
        put(&mut body, "phaseId", json!(phase));
    }
    if let Some((altitude, source)) = meta.get(&piece.atom_id) {
        put(&mut body, "altitude", json!(altitude));
        put(&mut body, "source", json!(source));
    }
    if crewmux_spec::context::is_seat_recap_address(&piece.address)
        && let Some(note) = notes.get(&refer)
        && let Some(written) = &note.written_at
    {
        put(&mut body, "writtenAt", json!(written));
    }
    if let Some(note) = notes.get(&refer) {
        put(
            &mut body,
            "provenance",
            json!({
                "nominalPath": note.nominal,
                "realPath": note.real,
                "escapesRoot": note.escapes,
            }),
        );
    }
    body
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().fold(String::new(), |mut out, byte| {
        push_hex(&mut out, *byte);
        out
    })
}

fn push_hex(out: &mut String, byte: u8) {
    const HEX: &str = "0123456789abcdef";
    let hi = usize::from(byte / 16);
    let lo = usize::from(byte % 16);
    if let Some(ch) = HEX.chars().nth(hi) {
        out.push(ch);
    }
    if let Some(ch) = HEX.chars().nth(lo) {
        out.push(ch);
    }
}

fn situation_of(raw: Option<&str>) -> Result<Situation, RouteError> {
    match raw.and_then(Situation::parse) {
        Some(situation) => Ok(situation),
        None => Err(RouteError::named(
            400,
            "invalid_situation",
            format!(
                "situation must be fresh | handover | post-compaction (got: {})",
                raw.unwrap_or("(missing)")
            ),
        )),
    }
}

fn runtime_of(raw: Option<&str>) -> Result<ProfileRuntime, RouteError> {
    match raw.and_then(ProfileRuntime::parse) {
        Some(runtime) => Ok(runtime),
        None => Err(RouteError::named(
            400,
            "invalid_runtime",
            format!(
                "runtime must be claude | codex (got: {})",
                raw.unwrap_or("(missing)")
            ),
        )),
    }
}

fn budget_of(raw: Option<&str>) -> Result<Option<u32>, RouteError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RouteError::named(
            400,
            "invalid_budget",
            format!("budget must be a non-negative integer (got: {raw})"),
        ));
    }
    let value = raw.parse::<u32>().map_err(|_| {
        RouteError::named(
            400,
            "invalid_budget",
            format!("budget must be a non-negative integer (got: {raw})"),
        )
    })?;
    Ok(Some(value))
}

fn select_profile<'a>(
    manifest: &'a PackManifest,
    requested: Option<&str>,
    refer: &str,
    situation: Situation,
    runtime: ProfileRuntime,
) -> Result<Option<&'a Profile>, RouteError> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    let Some(profile) = manifest
        .profiles
        .iter()
        .find(|profile| profile.id == requested)
    else {
        let available = manifest
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let available = if available.is_empty() {
            "(none)".to_owned()
        } else {
            available
        };
        return Err(RouteError::named(
            400,
            "profile_not_found",
            format!("pack '{refer}' declares no profile '{requested}' — available: {available}"),
        ));
    };
    if !profile.situations.contains(&situation) {
        return Err(RouteError::named(
            400,
            "profile_situation_mismatch",
            format!(
                "profile '{}' does not apply to situation '{}'",
                profile.id,
                situation.as_str()
            ),
        ));
    }
    if !profile.runtimes.contains(&runtime) {
        return Err(RouteError::named(
            400,
            "profile_runtime_mismatch",
            format!(
                "profile '{}' does not apply to runtime '{}'",
                profile.id,
                runtime.as_str()
            ),
        ));
    }
    Ok(Some(profile))
}

fn check_grant(query: &QueryView, profile: Option<&Profile>) -> Result<(), RouteError> {
    let Some(profile) = profile else {
        return Ok(());
    };
    let mut needs_seat = false;
    let mut needs_work = false;
    for phase in &profile.phases {
        for source in phase.context.iter().flatten() {
            match source {
                ProfileSource::Seat => needs_seat = true,
                ProfileSource::Project | ProfileSource::Mission | ProfileSource::Slice => {
                    needs_work = true;
                }
            }
        }
    }
    if needs_seat && (query.rig.is_none() || query.seat.is_none()) {
        return Err(RouteError::named(
            400,
            "profile_context_missing",
            format!(
                "profile '{}' needs seat context; pass both rig and seat",
                profile.id
            ),
        ));
    }
    if needs_work && (query.mission.is_none() || query.slice.is_none()) {
        return Err(RouteError::named(
            400,
            "profile_context_missing",
            format!(
                "profile '{}' needs situated project/mission/task context; pass both mission and slice",
                profile.id
            ),
        ));
    }
    Ok(())
}

fn seat_pair(query: &QueryView) -> Result<(String, String), RouteError> {
    match (query.rig.as_deref(), query.seat.as_deref()) {
        (Some(rig), Some(seat)) if is_safe_segment(rig) && is_safe_segment(seat) => {
            Ok((rig.to_owned(), seat.to_owned()))
        }
        _ => Err(RouteError::named(
            400,
            "invalid_seat_params",
            "rig and seat must BOTH be single bounded segments ([A-Za-z0-9][A-Za-z0-9._-]{0,63})",
        )),
    }
}

fn learned_atom() -> Atom {
    Atom {
        id: "profile-seat-learned".to_owned(),
        address: "seat:LEARNED.md".to_owned(),
        taxonomy: Taxonomy::Lore,
        regions: None,
        situations: vec![Situation::Fresh],
        purpose: Purpose::Width,
        runtime: AtomRuntime::Any,
        order: 1,
        requires: None,
        priority: Priority::Core,
        profile_only: false,
        probe: None,
    }
}
