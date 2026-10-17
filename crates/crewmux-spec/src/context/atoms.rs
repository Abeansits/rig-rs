// Derived from openrig packages/daemon/src/domain/context-packs/manifest-parser.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Install-atom schema. The requires walk is iterative so a deep chain stays
//! inside [`ManifestError`](super::ManifestError).

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde_json::{Map, Value};

use super::error::{ManifestCode, ManifestError};
use super::markdown::parse_address;
use super::source_ref::parse_source_ref;
use super::types::{Atom, AtomRuntime, Priority, Probe, Purpose, Region, Situation, Taxonomy};

const ATOM_KEYS: &[&str] = &[
    "id",
    "address",
    "taxonomy",
    "regions",
    "situations",
    "purpose",
    "runtime",
    "order",
    "requires",
    "priority",
    "profile_only",
    "probe",
];
const ATOM_KEY_LIST: &str = "id, address, taxonomy, regions, situations, purpose, runtime, order, requires, priority, profile_only, probe";
const PROBE_KEY_LIST: &str = "prompt, expect, expectedPatterns, rubric";
const MARKDOWN: &[&str] = &[".md", ".markdown"];

pub(super) fn parse_atoms(
    raw: &Value,
    files: &[super::types::PackFile],
    source_path: &str,
) -> Result<Vec<Atom>, ManifestError> {
    let Some(entries) = raw.as_array() else {
        return Err(invalid(
            source_path,
            format!("manifest at {source_path} 'atoms' must be an array"),
        ));
    };
    let declared: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let mut atoms = Vec::with_capacity(entries.len());
    let mut seen = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        atoms.push(one_atom(entry, index, source_path, &declared, &mut seen)?);
    }
    check_requires(&atoms, source_path)?;
    check_cycles(&atoms, source_path)?;
    Ok(atoms)
}

fn one_atom(
    entry: &Value,
    index: usize,
    source_path: &str,
    declared: &BTreeSet<&str>,
    seen: &mut BTreeSet<String>,
) -> Result<Atom, ManifestError> {
    let Some(object) = entry.as_object() else {
        return Err(atom_err(source_path, index, "must be an object"));
    };
    reject_unknown_keys(object, source_path, index)?;
    let id = atom_id(object, source_path, index)?;
    if !seen.insert(id.clone()) {
        return Err(atom_err(
            source_path,
            index,
            format!("duplicate atom id '{id}' — ids are the join key for order/requires/probes"),
        ));
    }
    let address = address_of(object, source_path, index, declared)?;
    let taxonomy = enum_field::<Taxonomy>(object.get("taxonomy"), "taxonomy", source_path, index)?;
    let purpose = enum_field::<Purpose>(object.get("purpose"), "purpose", source_path, index)?;
    let priority = enum_field::<Priority>(object.get("priority"), "priority", source_path, index)?;
    let runtime = runtime_of(object, source_path, index)?;
    let profile_only = profile_only_of(object, source_path, index)?;
    let situations = situations_of(object, source_path, index)?;
    let regions = regions_of(object, source_path, index)?;
    let order = order_of(object, source_path, index)?;
    let requires = requires_of(object, &id, source_path, index)?;
    let probe = probe_of(object, source_path, index)?;
    Ok(Atom {
        id,
        address,
        taxonomy,
        regions,
        situations,
        purpose,
        runtime,
        order,
        requires,
        priority,
        profile_only,
        probe,
    })
}

fn reject_unknown_keys(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<(), ManifestError> {
    for key in object.keys() {
        if ATOM_KEYS.contains(&key.as_str()) {
            continue;
        }
        let hint = near_miss(key)
            .map(|meant| format!(" — did you mean '{meant}'?"))
            .unwrap_or_default();
        return Err(atom_err(
            source_path,
            index,
            format!("unknown field '{key}'{hint} (allowed: {ATOM_KEY_LIST})"),
        ));
    }
    Ok(())
}

fn near_miss(key: &str) -> Option<&'static str> {
    match key {
        "require" => Some("requires"),
        "region" => Some("regions"),
        "situation" => Some("situations"),
        "probes" => Some("probe"),
        _ => None,
    }
}

fn atom_id(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<String, ManifestError> {
    let got = js_got(object.get("id"));
    match object.get("id").and_then(Value::as_str) {
        Some(id) if atom_slug(id) => Ok(id.to_owned()),
        _ => Err(atom_err(
            source_path,
            index,
            format!("'id' must be a stable slug matching [a-z0-9][a-z0-9-]{{0,63}} (got: {got})"),
        )),
    }
}

fn atom_slug(id: &str) -> bool {
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

fn address_of(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
    declared: &BTreeSet<&str>,
) -> Result<String, ManifestError> {
    let Some(address) = object
        .get("address")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return Err(atom_err(
            source_path,
            index,
            "'address' is required: file or file#H2-slug/H3-slug",
        ));
    };
    let parsed = parse_address(address).map_err(|err| {
        atom_err(
            source_path,
            index,
            format!("'address' is malformed — {}", err.message),
        )
    })?;
    let source = parse_source_ref(&parsed.refer)
        .map_err(|err| atom_err(source_path, index, format!("'address' — {}", err.message)))?;
    if source.kind.as_str() == "library" && !declared.contains(parsed.refer.as_str()) {
        return Err(atom_err(
            source_path,
            index,
            format!(
                "'address' references '{}', which is not a declared pack file — a library atom addresses INTO the pack's own files (tree sources use the project:/seat:/mission: prefix)",
                parsed.refer
            ),
        ));
    }
    if !parsed.header_path.is_empty() && !MARKDOWN.iter().any(|suffix| source.rel.ends_with(suffix))
    {
        return Err(atom_err(
            source_path,
            index,
            format!(
                "'address' uses a header path on '{}' — header addressing only applies to markdown files ({})",
                parsed.refer,
                MARKDOWN.join(", ")
            ),
        ));
    }
    Ok(address.to_owned())
}

fn runtime_of(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<AtomRuntime, ManifestError> {
    match object.get("runtime") {
        None => Ok(AtomRuntime::Any),
        Some(value) => enum_field::<AtomRuntime>(Some(value), "runtime", source_path, index),
    }
}

fn profile_only_of(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<bool, ManifestError> {
    match object.get("profile_only") {
        None => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(other) => Err(atom_err(
            source_path,
            index,
            format!(
                "'profile_only' must be a boolean when present (got: {})",
                js_got(Some(other))
            ),
        )),
    }
}

fn situations_of(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<Vec<Situation>, ManifestError> {
    let Some(raw) = object
        .get("situations")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    else {
        return Err(atom_err(
            source_path,
            index,
            format!(
                "'situations' must be a non-empty array of {} — it is the composition algebra's selector",
                "fresh | handover | post-compaction"
            ),
        ));
    };
    raw.iter()
        .map(|item| enum_field::<Situation>(Some(item), "situations", source_path, index))
        .collect()
}

fn regions_of(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<Option<Vec<Region>>, ManifestError> {
    let Some(raw) = object.get("regions") else {
        return Ok(None);
    };
    let Some(items) = raw.as_array() else {
        return Err(atom_err(
            source_path,
            index,
            "'regions' must be an array when present",
        ));
    };
    let regions = items
        .iter()
        .map(|item| enum_field::<Region>(Some(item), "regions", source_path, index))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(regions))
}

fn order_of(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<i64, ManifestError> {
    let got = js_got(object.get("order"));
    js_integer(object.get("order")).ok_or_else(|| {
        atom_err(
            source_path,
            index,
            format!("'order' must be an integer — walks are ordered and absorption depends on sequence (got: {got})"),
        )
    })
}

fn requires_of(
    object: &Map<String, Value>,
    id: &str,
    source_path: &str,
    index: usize,
) -> Result<Option<Vec<String>>, ManifestError> {
    let Some(raw) = object.get("requires") else {
        return Ok(None);
    };
    let Some(items) = raw.as_array() else {
        return Err(atom_err(
            source_path,
            index,
            "'requires' must be an array of atom ids when present",
        ));
    };
    let mut requires = Vec::with_capacity(items.len());
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(atom_err(
                source_path,
                index,
                "'requires' must be an array of atom ids when present",
            ));
        };
        requires.push(text.to_owned());
    }
    if requires.iter().any(|req| req == id) {
        return Err(atom_err(
            source_path,
            index,
            format!("atom '{id}' requires itself"),
        ));
    }
    Ok(Some(requires))
}

fn probe_of(
    object: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<Option<Probe>, ManifestError> {
    let Some(raw) = object.get("probe") else {
        return Ok(None);
    };
    let Some(probe) = raw.as_object() else {
        return Err(atom_err(
            source_path,
            index,
            "'probe' must be an object { prompt, expect, expectedPatterns?, rubric? }",
        ));
    };
    for key in probe.keys() {
        if matches!(
            key.as_str(),
            "prompt" | "expect" | "expectedPatterns" | "rubric"
        ) {
            continue;
        }
        let hint = if key == "rubrics" {
            " — did you mean 'rubric'?"
        } else {
            ""
        };
        return Err(atom_err(
            source_path,
            index,
            format!("'probe' has unknown field '{key}'{hint} (allowed: {PROBE_KEY_LIST})"),
        ));
    }
    let prompt = probe.get("prompt").and_then(Value::as_str).unwrap_or("");
    let expect = probe.get("expect").and_then(Value::as_str).unwrap_or("");
    if prompt.is_empty() || expect.is_empty() {
        return Err(atom_err(
            source_path,
            index,
            "'probe' needs BOTH a natural 'prompt' and the 'expect'ed observable behavior — acceptance is changed behavior, never files delivered",
        ));
    }
    let expected_patterns = patterns_of(probe, source_path, index)?;
    let rubric = match probe.get("rubric") {
        None => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => {
            return Err(atom_err(
                source_path,
                index,
                "'probe.rubric' must be a string when present",
            ));
        }
    };
    Ok(Some(Probe {
        prompt: prompt.to_owned(),
        expect: expect.to_owned(),
        expected_patterns,
        rubric,
    }))
}

fn patterns_of(
    probe: &Map<String, Value>,
    source_path: &str,
    index: usize,
) -> Result<Option<Vec<String>>, ManifestError> {
    let Some(raw) = probe.get("expectedPatterns") else {
        return Ok(None);
    };
    let Some(items) = raw.as_array() else {
        return Err(atom_err(
            source_path,
            index,
            "'probe.expectedPatterns' must be an array of regex source strings when present",
        ));
    };
    let mut patterns = Vec::with_capacity(items.len());
    for item in items {
        let Some(src) = item.as_str() else {
            return Err(atom_err(
                source_path,
                index,
                "'probe.expectedPatterns' must be an array of regex source strings when present",
            ));
        };
        if Regex::new(src).is_err() {
            return Err(atom_err(
                source_path,
                index,
                format!("'probe.expectedPatterns' entry '{src}' is not a compilable regex source"),
            ));
        }
        patterns.push(src.to_owned());
    }
    Ok(Some(patterns))
}

fn check_requires(atoms: &[Atom], source_path: &str) -> Result<(), ManifestError> {
    let ids: BTreeSet<&str> = atoms.iter().map(|atom| atom.id.as_str()).collect();
    for (index, atom) in atoms.iter().enumerate() {
        for req in atom.requires.iter().flatten() {
            if !ids.contains(req.as_str()) {
                return Err(atom_err(
                    source_path,
                    index,
                    format!(
                        "atom '{}' requires '{req}', which no atom declares",
                        atom.id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn check_cycles(atoms: &[Atom], source_path: &str) -> Result<(), ManifestError> {
    let by_id: BTreeMap<&str, &Atom> = atoms.iter().map(|atom| (atom.id.as_str(), atom)).collect();
    let mut state: BTreeMap<&str, bool> = BTreeMap::new();
    for root in atoms {
        if state.get(root.id.as_str()) == Some(&true) {
            continue;
        }
        let mut stack: Vec<(&str, usize)> = vec![(root.id.as_str(), 0)];
        state.insert(root.id.as_str(), false);
        while let Some((id, next)) = stack.last().copied() {
            let reqs = by_id
                .get(id)
                .and_then(|atom| atom.requires.as_deref())
                .unwrap_or(&[]);
            if next >= reqs.len() {
                state.insert(id, true);
                stack.pop();
                continue;
            }
            let Some(req) = reqs.get(next).map(String::as_str) else {
                break;
            };
            if let Some(frame) = stack.last_mut() {
                frame.1 = frame.1.saturating_add(1);
            }
            match state.get(req) {
                Some(true) => {}
                Some(false) => {
                    let trail: Vec<&str> = stack.iter().map(|(id, _)| *id).collect();
                    let start = trail.iter().position(|id| *id == req).unwrap_or(0);
                    let mut cycle: Vec<&str> = trail.iter().skip(start).copied().collect();
                    cycle.push(req);
                    return Err(invalid(
                        source_path,
                        format!(
                            "manifest at {source_path} atoms: requires cycle {} — no subset profile could ever close over it",
                            cycle.join(" -> ")
                        ),
                    ));
                }
                None => {
                    state.insert(req, false);
                    stack.push((req, 0));
                }
            }
        }
    }
    Ok(())
}

fn enum_field<T>(
    value: Option<&Value>,
    field: &str,
    source_path: &str,
    index: usize,
) -> Result<T, ManifestError>
where
    T: Token,
{
    let text = value.and_then(Value::as_str).unwrap_or("");
    T::from_token(text).ok_or_else(|| {
        atom_err(
            source_path,
            index,
            format!(
                "'{field}' must be one of {} (got: {})",
                T::list(),
                js_got(value)
            ),
        )
    })
}

trait Token: Sized {
    fn from_token(text: &str) -> Option<Self>;
    fn list() -> &'static str;
}

macro_rules! token_list {
    ($ty:ty, $list:literal) => {
        impl Token for $ty {
            fn from_token(text: &str) -> Option<Self> {
                <$ty>::parse(text)
            }
            fn list() -> &'static str {
                $list
            }
        }
    };
}

token_list!(Taxonomy, "world | lore | skills | mission");
token_list!(Purpose, "depth | width");
token_list!(Priority, "optional | recommended | core");
token_list!(AtomRuntime, "claude | codex | any");
token_list!(Situation, "fresh | handover | post-compaction");
token_list!(
    Region,
    "identity | ontology | terrain | actors | laws | history | state | affordances"
);

fn js_integer(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_number()?;
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    if let Some(value) = number.as_u64() {
        return i64::try_from(value).ok();
    }
    let value = number.as_f64()?;
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }
    format!("{value:.0}").parse().ok()
}

fn js_got(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned()),
    }
}

fn atom_err(source_path: &str, index: usize, detail: impl AsRef<str>) -> ManifestError {
    ManifestError::new(
        ManifestCode::Invalid,
        format!(
            "manifest at {source_path} atoms[{index}]: {}",
            detail.as_ref()
        ),
    )
}

fn invalid(source_path: &str, message: String) -> ManifestError {
    let _ = source_path;
    ManifestError::new(ManifestCode::Invalid, message)
}
