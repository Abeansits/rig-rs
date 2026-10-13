// Derived from openrig packages/daemon/src/routes/context-packs.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Bounded legacy project → mission → slice walk for the profile route.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crewmux_orchestration::context_packs::{ContextAtoms, SourceRoots};
use crewmux_spec::context::{
    Atom, AtomRuntime, Priority, Profile, Purpose, Situation, SourceKind, Taxonomy,
    is_safe_segment, load_yaml, parse_address, source_kind_for_address,
};
use serde_json::Value;

use crate::context_http::RouteError;

pub(crate) struct LegacyQuery<'a> {
    pub refer: &'a str,
    pub situation: Situation,
    pub profile: Option<&'a Profile>,
    pub mission: &'a str,
    pub slice: Option<&'a str>,
    pub slices: &'a Path,
    pub workspace: &'a Path,
}

pub(crate) fn apply(
    atoms: &mut Vec<Atom>,
    context: &mut ContextAtoms,
    meta: &mut BTreeMap<String, (&'static str, &'static str)>,
    warnings: &mut Vec<String>,
    roots: &mut SourceRoots,
    query: &LegacyQuery<'_>,
) -> Result<(), RouteError> {
    if !is_safe_segment(query.mission) {
        return Err(RouteError::named(
            400,
            "invalid_mission_param",
            "mission must be a single bounded segment ([A-Za-z0-9][A-Za-z0-9._-]{0,63})",
        ));
    }
    if let Some(slice) = query.slice
        && !is_safe_segment(slice)
    {
        return Err(RouteError::named(
            400,
            "invalid_slice_param",
            "slice must be a single bounded segment ([A-Za-z0-9][A-Za-z0-9._-]{0,63})",
        ));
    }
    if query.slice.is_some() && query.situation != Situation::Fresh {
        return Err(RouteError::named(
            400,
            "legacy_default_fresh_only",
            "the bounded legacy default work walk is available only for a fresh profile",
        ));
    }
    roots.mission = Some(query.slices.join(query.mission));
    let authored = atoms.iter().any(|atom| {
        matches!(
            source_kind_for_address(&atom.address).ok(),
            Some(SourceKind::Project | SourceKind::Mission)
        )
    });
    if query.slice.is_none() && !authored && query.profile.is_none() {
        return Err(RouteError::named(
            400,
            "slice_required",
            format!(
                "pack '{}' declares no project: or mission: atoms, so --mission alone would be a no-op; pass the exact --slice to request the legacy default work walk",
                query.refer
            ),
        ));
    }
    let Some(slice) = query.slice else {
        return Ok(());
    };
    let missions = query.workspace.join("missions");
    if normalize(query.slices) != normalize(&missions) {
        return Err(RouteError::named(
            422,
            "legacy_workspace_required",
            format!(
                "legacy default composition requires workspace.slices_root to be <workspace.root>/missions; got {}",
                query.slices.display()
            ),
        ));
    }
    roots.project = Some(query.workspace.to_path_buf());
    stamp(atoms, context, meta, warnings, query, slice)
}

fn stamp(
    atoms: &mut Vec<Atom>,
    context: &mut ContextAtoms,
    meta: &mut BTreeMap<String, (&'static str, &'static str)>,
    warnings: &mut Vec<String>,
    query: &LegacyQuery<'_>,
    slice: &str,
) -> Result<(), RouteError> {
    let (intent, intent_source, extra, extra_warnings) = project_intent(query.workspace);
    warnings.extend(extra_warnings);
    let prefix = if query.profile.is_some() {
        "profile"
    } else {
        "legacy-default"
    };
    let max_order = atoms.iter().map(|atom| atom.order).fold(i64::MIN, i64::max);
    let mut project = Vec::new();
    project.push(work_atom(
        &format!("{prefix}-project-spec"),
        &format!("project:{intent}"),
        max_order.saturating_add(1),
        None,
        Purpose::Depth,
    ));
    for (index, address) in extra.iter().enumerate() {
        let id = format!("{prefix}-project-context-{}", index + 1);
        let requires = if index == 0 {
            format!("{prefix}-project-spec")
        } else {
            format!("{prefix}-project-context-{index}")
        };
        project.push(work_atom(
            &id,
            &format!("project:{address}"),
            max_order
                .saturating_add(i64::try_from(index).unwrap_or(0))
                .saturating_add(2),
            Some(vec![requires]),
            Purpose::Depth,
        ));
    }
    let final_order = match project.last() {
        Some(atom) => atom.order,
        None => max_order,
    };
    let mission_spec = work_atom(
        &format!("{prefix}-mission-spec"),
        "mission:SPEC.md",
        final_order.saturating_add(1),
        project.last().map(|atom| vec![atom.id.clone()]),
        Purpose::Depth,
    );
    let slice_spec = work_atom(
        &format!("{prefix}-slice-spec"),
        &format!("mission:slices/{slice}/SPEC.md"),
        final_order.saturating_add(2),
        Some(vec![mission_spec.id.clone()]),
        Purpose::Depth,
    );
    let mut defaults = project.clone();
    defaults.push(mission_spec.clone());
    defaults.push(slice_spec.clone());
    if let Some(collision) = defaults
        .iter()
        .find(|atom| atoms.iter().any(|existing| existing.id == atom.id))
    {
        return Err(RouteError::named(
            422,
            "default_atom_conflict",
            format!(
                "pack '{}' already declares reserved atom id '{}'",
                query.refer, collision.id
            ),
        ));
    }
    meta.insert(format!("{prefix}-project-spec"), ("project", intent_source));
    for (index, _) in extra.iter().enumerate() {
        meta.insert(
            format!("{prefix}-project-context-{}", index + 1),
            ("project", "manifest"),
        );
    }
    meta.insert(format!("{prefix}-mission-spec"), ("mission", "default"));
    meta.insert(format!("{prefix}-slice-spec"), ("slice", "default"));
    if query.profile.is_some() {
        place_named(context, meta, project, &mission_spec, &slice_spec, slice);
    } else {
        atoms.extend(defaults);
    }
    Ok(())
}

fn place_named(
    context: &mut ContextAtoms,
    meta: &mut BTreeMap<String, (&'static str, &'static str)>,
    project: Vec<Atom>,
    mission_spec: &Atom,
    slice_spec: &Atom,
    slice: &str,
) {
    context.mission = vec![
        mission_spec.clone(),
        work_atom(
            "profile-mission-arrangement",
            "mission:mission.yaml",
            mission_spec.order.saturating_add(1),
            None,
            Purpose::Width,
        ),
        work_atom(
            "profile-mission-progress",
            "mission:PROGRESS.md",
            mission_spec.order.saturating_add(2),
            None,
            Purpose::Width,
        ),
    ];
    context.slice = vec![
        slice_spec.clone(),
        work_atom(
            "profile-slice-progress",
            &format!("mission:slices/{slice}/PROGRESS.md"),
            slice_spec.order.saturating_add(1),
            None,
            Purpose::Width,
        ),
    ];
    context.project = project;
    meta.insert(
        "profile-mission-arrangement".to_owned(),
        ("mission", "default"),
    );
    meta.insert(
        "profile-mission-progress".to_owned(),
        ("mission", "default"),
    );
    meta.insert("profile-slice-progress".to_owned(), ("slice", "default"));
}

fn project_intent(workspace: &Path) -> (String, &'static str, Vec<String>, Vec<String>) {
    let path = workspace.join("project.yaml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ("SPEC.md".to_owned(), "default", Vec::new(), Vec::new());
    };
    let Ok(value) = load_yaml(&text, &path.display().to_string()) else {
        return ("SPEC.md".to_owned(), "default", Vec::new(), Vec::new());
    };
    let mut warnings = Vec::new();
    let mut intent = "SPEC.md".to_owned();
    let mut source = "default";
    let raw_intent = value.pointer("/install/intent");
    if let Some(raw) = raw_intent {
        if relative_markdown(raw) {
            if let Some(text) = raw.as_str() {
                text.clone_into(&mut intent);
            }
            source = "manifest";
        } else {
            warnings.push(
                "project.yaml: optional install.intent must be a relative Markdown address; ignored the invalid value and kept the baseline work install."
                    .to_owned(),
            );
        }
    }
    let mut extra = Vec::new();
    if let Some(raw) = value.pointer("/install/context") {
        if raw
            .as_array()
            .is_some_and(|items| items.iter().all(relative_markdown))
        {
            extra.extend(
                raw.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.as_str().map(str::to_owned)),
            );
        } else {
            warnings.push(
                "project.yaml: optional install.context must be a list of relative Markdown addresses; ignored the invalid value and kept the baseline work install."
                    .to_owned(),
            );
        }
    }
    (intent, source, extra, warnings)
}

fn relative_markdown(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let Ok(parsed) = parse_address(text) else {
        return false;
    };
    if parsed.refer.is_empty() || parsed.refer.starts_with('/') {
        return false;
    }
    if parsed
        .refer
        .split(['\\', '/'])
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return false;
    }
    match parsed.refer.rsplit_once('.') {
        Some((_, ext)) => {
            let ext = ext.to_ascii_lowercase();
            ext == "md" || ext == "markdown"
        }
        None => false,
    }
}

fn work_atom(
    id: &str,
    address: &str,
    order: i64,
    requires: Option<Vec<String>>,
    purpose: Purpose,
) -> Atom {
    Atom {
        id: id.to_owned(),
        address: address.to_owned(),
        taxonomy: Taxonomy::Mission,
        regions: None,
        situations: vec![Situation::Fresh],
        purpose,
        runtime: AtomRuntime::Any,
        order,
        requires,
        priority: Priority::Core,
        profile_only: false,
        probe: None,
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::Normal(part) => out.push(part),
        }
    }
    out
}
