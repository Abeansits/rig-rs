// Derived from openrig packages/daemon/src/domain/context-packs/manifest-parser.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Named install profiles.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::error::{ManifestCode, ManifestError};
use super::types::{
    Atom, AtomRuntime, Profile, ProfilePhase, ProfileRuntime, ProfileSource, Situation,
};

const PROFILE_KEYS: &[&str] = &["id", "situations", "runtimes", "phases"];
const PROFILE_KEY_LIST: &str = "id, situations, runtimes, phases";
const PHASE_KEYS: &[&str] = &["id", "atoms", "context"];
const PHASE_KEY_LIST: &str = "id, atoms, context";

pub(super) fn parse_profiles(
    raw: &Value,
    atoms: &[Atom],
    source_path: &str,
) -> Result<Vec<Profile>, ManifestError> {
    let Some(entries) = raw.as_array() else {
        return Err(err(
            source_path,
            format!("manifest at {source_path} 'profiles' must be an array"),
        ));
    };
    if atoms.is_empty() && !entries.is_empty() {
        return Err(err(
            source_path,
            format!(
                "manifest at {source_path} declares profiles but no atoms — atom phases need one canonical source graph"
            ),
        ));
    }
    let by_id: BTreeMap<&str, &Atom> = atoms.iter().map(|atom| (atom.id.as_str(), atom)).collect();
    let mut seen = BTreeSet::new();
    let mut profiles = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        profiles.push(one_profile(entry, index, source_path, &by_id, &mut seen)?);
    }
    Ok(profiles)
}

fn one_profile(
    entry: &Value,
    index: usize,
    source_path: &str,
    by_id: &BTreeMap<&str, &Atom>,
    seen: &mut BTreeSet<String>,
) -> Result<Profile, ManifestError> {
    let Some(object) = entry.as_object() else {
        return Err(profile_err(source_path, index, "must be an object"));
    };
    reject_keys(
        object,
        PROFILE_KEYS,
        PROFILE_KEY_LIST,
        source_path,
        index,
        None,
    )?;
    let id = slug(
        object.get("id"),
        source_path,
        index,
        "'id' must be a stable slug matching [a-z0-9][a-z0-9-]{0,63}",
    )?;
    if !seen.insert(id.clone()) {
        return Err(profile_err(
            source_path,
            index,
            format!("duplicate profile id '{id}'"),
        ));
    }
    let situations = unique_enums::<Situation>(
        object.get("situations"),
        "situations",
        "fresh | handover | post-compaction",
        source_path,
        index,
    )?;
    let runtimes = unique_enums::<ProfileRuntime>(
        object.get("runtimes"),
        "runtimes",
        "claude | codex",
        source_path,
        index,
    )?;
    let walk = Walk {
        source_path,
        index,
        profile_id: &id,
        situations: &situations,
        runtimes: &runtimes,
        by_id,
    };
    let phases = phases_of(object.get("phases"), &walk)?;
    Ok(Profile {
        id,
        situations,
        runtimes,
        phases,
    })
}

struct Walk<'a> {
    source_path: &'a str,
    index: usize,
    profile_id: &'a str,
    situations: &'a [Situation],
    runtimes: &'a [ProfileRuntime],
    by_id: &'a BTreeMap<&'a str, &'a Atom>,
}

fn phases_of(raw: Option<&Value>, walk: &Walk<'_>) -> Result<Vec<ProfilePhase>, ManifestError> {
    let Some(entries) = raw
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return Err(profile_err(
            walk.source_path,
            walk.index,
            "'phases' must be a non-empty array",
        ));
    };
    let mut phases = Vec::with_capacity(entries.len());
    let mut seen_ids = BTreeSet::new();
    let mut selected_atoms = BTreeSet::new();
    let mut selected_context = BTreeSet::new();
    for (phase_index, entry) in entries.iter().enumerate() {
        phases.push(one_phase(
            entry,
            phase_index,
            walk,
            &mut seen_ids,
            &mut selected_atoms,
            &mut selected_context,
        )?);
    }
    Ok(phases)
}

fn one_phase(
    entry: &Value,
    phase_index: usize,
    walk: &Walk<'_>,
    seen_ids: &mut BTreeSet<String>,
    selected_atoms: &mut BTreeSet<String>,
    selected_context: &mut BTreeSet<ProfileSource>,
) -> Result<ProfilePhase, ManifestError> {
    let source_path = walk.source_path;
    let index = walk.index;
    let Some(object) = entry.as_object() else {
        return Err(profile_err(
            source_path,
            index,
            format!("phases[{phase_index}] must be an object"),
        ));
    };
    reject_keys(
        object,
        PHASE_KEYS,
        PHASE_KEY_LIST,
        source_path,
        index,
        Some(phase_index),
    )?;
    let id = phase_id(object.get("id"), source_path, index, phase_index)?;
    if !seen_ids.insert(id.clone()) {
        return Err(profile_err(
            source_path,
            index,
            format!("duplicate phase id '{id}'"),
        ));
    }
    let has_atoms = object.contains_key("atoms");
    let has_context = object.contains_key("context");
    if has_atoms == has_context {
        return Err(profile_err(
            source_path,
            index,
            format!("phases[{phase_index}] '{id}' must declare exactly one of atoms or context"),
        ));
    }
    if has_atoms {
        let atoms = phase_atoms(object.get("atoms"), phase_index, &id, walk, selected_atoms)?;
        return Ok(ProfilePhase {
            id,
            atoms: Some(atoms),
            context: None,
        });
    }
    let context = phase_context(object.get("context"), phase_index, walk, selected_context)?;
    Ok(ProfilePhase {
        id,
        atoms: None,
        context: Some(context),
    })
}

fn phase_atoms(
    raw: Option<&Value>,
    phase_index: usize,
    phase_id: &str,
    walk: &Walk<'_>,
    selected: &mut BTreeSet<String>,
) -> Result<Vec<String>, ManifestError> {
    let source_path = walk.source_path;
    let index = walk.index;
    let Some(items) = raw
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return Err(profile_err(
            source_path,
            index,
            format!("phases[{phase_index}].atoms must be a non-empty array of atom ids"),
        ));
    };
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let Some(atom_id) = item.as_str() else {
            return Err(profile_err(
                source_path,
                index,
                format!("phases[{phase_index}].atoms must be a non-empty array of atom ids"),
            ));
        };
        ids.push(atom_id.to_owned());
    }
    for atom_id in &ids {
        let Some(atom) = walk.by_id.get(atom_id.as_str()) else {
            return Err(profile_err(
                source_path,
                index,
                format!("phase '{phase_id}' references missing atom '{atom_id}'"),
            ));
        };
        if !selected.insert(atom_id.clone()) {
            return Err(profile_err(
                source_path,
                index,
                format!(
                    "atom '{atom_id}' appears more than once in profile '{}'",
                    walk.profile_id
                ),
            ));
        }
        check_eligibility(atom, atom_id, walk, &ids, selected)?;
    }
    Ok(ids)
}

fn check_eligibility(
    atom: &Atom,
    atom_id: &str,
    walk: &Walk<'_>,
    phase_ids: &[String],
    selected: &BTreeSet<String>,
) -> Result<(), ManifestError> {
    let source_path = walk.source_path;
    let index = walk.index;
    for situation in walk.situations {
        if !atom.situations.contains(situation) {
            return Err(profile_err(
                source_path,
                index,
                format!(
                    "atom '{atom_id}' is not eligible for profile situation '{}'",
                    situation.as_str()
                ),
            ));
        }
    }
    for runtime in walk.runtimes {
        let compatible =
            atom.runtime == AtomRuntime::Any || atom.runtime.as_str() == runtime.as_str();
        if !compatible {
            return Err(profile_err(
                source_path,
                index,
                format!(
                    "atom '{atom_id}' is runtime={}, incompatible with profile runtime '{}'",
                    atom.runtime.as_str(),
                    runtime.as_str()
                ),
            ));
        }
    }
    for required in atom.requires.iter().flatten() {
        let in_phase = phase_ids.iter().any(|id| id == required);
        if !selected.contains(required) && !in_phase {
            return Err(profile_err(
                source_path,
                index,
                format!(
                    "atom '{atom_id}' requires '{required}', which must appear in the same or an earlier phase"
                ),
            ));
        }
    }
    Ok(())
}

fn phase_context(
    raw: Option<&Value>,
    phase_index: usize,
    walk: &Walk<'_>,
    selected: &mut BTreeSet<ProfileSource>,
) -> Result<Vec<ProfileSource>, ManifestError> {
    let source_path = walk.source_path;
    let index = walk.index;
    let Some(items) = raw
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return Err(profile_err(
            source_path,
            index,
            format!(
                "phases[{phase_index}].context must be a non-empty array of project | mission | seat | slice"
            ),
        ));
    };
    let mut context = Vec::with_capacity(items.len());
    for item in items {
        let Some(source) = item.as_str().and_then(ProfileSource::parse) else {
            return Err(profile_err(
                source_path,
                index,
                format!(
                    "'context' must be one of project | mission | seat | slice (got: {})",
                    js_got(Some(item))
                ),
            ));
        };
        if !selected.insert(source) {
            return Err(profile_err(
                source_path,
                index,
                format!(
                    "context source '{}' appears more than once in profile '{}'",
                    source.as_str(),
                    walk.profile_id
                ),
            ));
        }
        context.push(source);
    }
    Ok(context)
}

fn unique_enums<T: Copy + Ord + ProfileToken>(
    raw: Option<&Value>,
    field: &str,
    allowed: &str,
    source_path: &str,
    index: usize,
) -> Result<Vec<T>, ManifestError> {
    let Some(items) = raw
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return Err(profile_err(
            source_path,
            index,
            format!("'{field}' must be a non-empty array of {allowed}"),
        ));
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Some(value) = item.as_str().and_then(T::from_token) else {
            return Err(profile_err(
                source_path,
                index,
                format!(
                    "'{field}' must be one of {allowed} (got: {})",
                    js_got(Some(item))
                ),
            ));
        };
        values.push(value);
    }
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(profile_err(
            source_path,
            index,
            format!("'{field}' must not repeat values"),
        ));
    }
    Ok(values)
}

trait ProfileToken: Sized {
    fn from_token(text: &str) -> Option<Self>;
}

impl ProfileToken for Situation {
    fn from_token(text: &str) -> Option<Self> {
        Self::parse(text)
    }
}

impl ProfileToken for ProfileRuntime {
    fn from_token(text: &str) -> Option<Self> {
        Self::parse(text)
    }
}

fn phase_id(
    value: Option<&Value>,
    source_path: &str,
    index: usize,
    phase_index: usize,
) -> Result<String, ManifestError> {
    match value.and_then(Value::as_str) {
        Some(id) if slug_ok(id) => Ok(id.to_owned()),
        _ => Err(profile_err(
            source_path,
            index,
            format!(
                "phases[{phase_index}].id must be a stable slug (got: {})",
                js_got(value)
            ),
        )),
    }
}

fn slug(
    value: Option<&Value>,
    source_path: &str,
    index: usize,
    detail: &str,
) -> Result<String, ManifestError> {
    match value.and_then(Value::as_str) {
        Some(id) if slug_ok(id) => Ok(id.to_owned()),
        _ => Err(profile_err(
            source_path,
            index,
            format!("{detail} (got: {})", js_got(value)),
        )),
    }
}

fn slug_ok(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    let mut len = 1usize;
    for ch in chars {
        len = len.saturating_add(1);
        if len > 64 || !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') {
            return false;
        }
    }
    true
}

fn reject_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    list: &str,
    source_path: &str,
    index: usize,
    phase: Option<usize>,
) -> Result<(), ManifestError> {
    for key in object.keys() {
        if allowed.iter().any(|item| *item == key) {
            continue;
        }
        let detail = match phase {
            Some(phase) => format!("phases[{phase}] has unknown field '{key}' (allowed: {list})"),
            None => format!("unknown field '{key}' (allowed: {list})"),
        };
        return Err(profile_err(source_path, index, detail));
    }
    Ok(())
}

fn js_got(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned()),
    }
}

fn profile_err(source_path: &str, index: usize, detail: impl AsRef<str>) -> ManifestError {
    ManifestError::new(
        ManifestCode::Invalid,
        format!(
            "manifest at {source_path} profiles[{index}]: {}",
            detail.as_ref()
        ),
    )
}

fn err(_source_path: &str, message: String) -> ManifestError {
    ManifestError::new(ManifestCode::Invalid, message)
}
