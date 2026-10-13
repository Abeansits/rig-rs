// Derived from openrig packages/daemon/src/domain/context-packs/profile-composer.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Situation algebra. Budgets report overage and never truncate.

use std::collections::{BTreeMap, BTreeSet};

use crewmux_spec::context::{
    Atom, Priority, Profile, ProfileRuntime, ProfileSource, Situation, is_seat_recap_address,
    parse_address, resolve_address, source_kind_for_address,
};

use super::read::ReadFail;
use super::tokens::estimate_tokens;

/// Composer failure. The walk stops rather than thinning.
#[derive(Debug)]
pub struct ComposeError {
    /// TypeScript sentence.
    pub message: String,
}

/// One resolved piece.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposedPiece {
    /// Atom id.
    pub atom_id: String,
    /// Address.
    pub address: String,
    /// Source label.
    pub source_kind: String,
    /// Walk position.
    pub order: i64,
    /// Drop tier.
    pub priority: Priority,
    /// Resolved bytes.
    pub text: String,
    /// Token estimate.
    pub estimated_tokens: u32,
    /// Named-profile phase, when this piece came from one.
    pub phase_id: Option<String>,
}

/// Budget report. Present only when the budget binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetReport {
    /// Limit.
    pub limit_tokens: u32,
    /// How far over.
    pub overage_tokens: u32,
    /// Drop order: optional, then recommended, then core; larger first.
    pub drop_candidates: Vec<DropCandidate>,
}

/// One drop candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropCandidate {
    /// Atom id.
    pub atom_id: String,
    /// Priority.
    pub priority: Priority,
    /// Tokens.
    pub estimated_tokens: u32,
}

/// A composed profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposedProfile {
    /// Situation.
    pub situation: Situation,
    /// Runtime.
    pub runtime: ProfileRuntime,
    /// Ordered pieces.
    pub pieces: Vec<ComposedPiece>,
    /// Sum of piece estimates.
    pub total_estimated_tokens: u32,
    /// Named profile id.
    pub profile_id: Option<String>,
    /// Named phases.
    pub phases: Vec<ComposedPhase>,
    /// Absent seat recaps skipped on post-compaction.
    pub skipped: Vec<(String, String)>,
    /// Budget, when it binds.
    pub budget: Option<BudgetReport>,
}

/// One named phase after resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposedPhase {
    /// Phase id.
    pub id: String,
    /// `atoms` or `context`.
    pub kind: &'static str,
    /// Context sources, when this phase inserts them.
    pub sources: Vec<ProfileSource>,
    /// Pieces.
    pub pieces: Vec<ComposedPiece>,
    /// Sum.
    pub estimated_tokens: u32,
}

/// Situated atoms supplied by the caller for a named profile.
#[derive(Clone, Debug, Default)]
pub struct ContextAtoms {
    /// Project phase.
    pub project: Vec<Atom>,
    /// Mission phase.
    pub mission: Vec<Atom>,
    /// Seat phase.
    pub seat: Vec<Atom>,
    /// Slice phase.
    pub slice: Vec<Atom>,
}

impl ContextAtoms {
    fn get(&self, source: ProfileSource) -> &[Atom] {
        match source {
            ProfileSource::Project => &self.project,
            ProfileSource::Mission => &self.mission,
            ProfileSource::Seat => &self.seat,
            ProfileSource::Slice => &self.slice,
        }
    }
}

/// Compose the legacy situation walk.
///
/// # Errors
///
/// Returns [`ComposeError`] when the closure cannot close or a piece cannot resolve.
pub fn compose_profile(
    atoms: &[Atom],
    situation: Situation,
    runtime: ProfileRuntime,
    budget_tokens: Option<u32>,
    read_file: &mut dyn FnMut(&str) -> Result<String, ReadFail>,
) -> Result<ComposedProfile, ComposeError> {
    let selected = select(atoms, situation, runtime)?;
    let mut walk: Vec<&Atom> = selected.values().copied().collect();
    walk.sort_by(|left, right| left.order.cmp(&right.order).then(left.id.cmp(&right.id)));
    let mut skipped = Vec::new();
    let pieces = resolve_pieces(&walk, situation, read_file, None, &mut skipped)?;
    Ok(finish(Done {
        situation,
        runtime,
        profile_id: None,
        phases: Vec::new(),
        pieces,
        skipped,
        budget_tokens,
    }))
}

/// Inputs for [`compose_named_profile`].
#[derive(Clone, Copy)]
pub struct NamedCompose<'a> {
    /// The pack's atom graph.
    pub atoms: &'a [Atom],
    /// The selected manifest profile.
    pub profile: &'a Profile,
    /// Situation being composed.
    pub situation: Situation,
    /// Runtime being composed.
    pub runtime: ProfileRuntime,
    /// Situated atoms supplied by the caller.
    pub context: &'a ContextAtoms,
    /// Token budget. Overage is reported and nothing is truncated.
    pub budget_tokens: Option<u32>,
}

/// Compose one named profile.
///
/// # Errors
///
/// Returns [`ComposeError`] when the profile does not apply or a phase cannot resolve.
pub fn compose_named_profile(
    request: &NamedCompose<'_>,
    read_file: &mut dyn FnMut(&str) -> Result<String, ReadFail>,
) -> Result<ComposedProfile, ComposeError> {
    let NamedCompose {
        atoms,
        profile,
        situation,
        runtime,
        context,
        budget_tokens,
    } = *request;
    if !profile.situations.contains(&situation) {
        return Err(fail(format!(
            "profile '{}' does not apply to situation '{}'",
            profile.id,
            situation.as_str()
        )));
    }
    if !profile.runtimes.contains(&runtime) {
        return Err(fail(format!(
            "profile '{}' does not apply to runtime '{}'",
            profile.id,
            runtime.as_str()
        )));
    }
    let by_id: BTreeMap<&str, &Atom> = atoms.iter().map(|atom| (atom.id.as_str(), atom)).collect();
    let mut skipped = Vec::new();
    let mut phases = Vec::new();
    for phase in &profile.phases {
        let job = PhaseJob {
            phase,
            profile,
            by_id: &by_id,
            context,
            situation,
        };
        phases.push(one_phase(&job, read_file, &mut skipped)?);
    }
    let pieces = phases
        .iter()
        .flat_map(|phase| phase.pieces.clone())
        .collect();
    Ok(finish(Done {
        situation,
        runtime,
        profile_id: Some(profile.id.clone()),
        phases,
        pieces,
        skipped,
        budget_tokens,
    }))
}

struct PhaseJob<'a> {
    phase: &'a crewmux_spec::context::ProfilePhase,
    profile: &'a Profile,
    by_id: &'a BTreeMap<&'a str, &'a Atom>,
    context: &'a ContextAtoms,
    situation: Situation,
}

fn one_phase(
    job: &PhaseJob<'_>,
    read_file: &mut dyn FnMut(&str) -> Result<String, ReadFail>,
    skipped: &mut Vec<(String, String)>,
) -> Result<ComposedPhase, ComposeError> {
    let phase = job.phase;
    let profile = job.profile;
    let by_id = job.by_id;
    let context = job.context;
    let situation = job.situation;
    let (kind, selected, sources) = if let Some(ids) = &phase.atoms {
        let mut selected = Vec::new();
        for atom_id in ids {
            let Some(atom) = by_id.get(atom_id.as_str()) else {
                return Err(fail(format!(
                    "profile '{}' phase '{}' references missing atom '{atom_id}'",
                    profile.id, phase.id
                )));
            };
            selected.push(*atom);
        }
        ("atoms", selected, Vec::new())
    } else {
        let mut selected = Vec::new();
        let sources = phase.context.clone().unwrap_or_default();
        for source in &sources {
            let source_atoms = context.get(*source);
            if source_atoms.is_empty() {
                return Err(fail(format!(
                    "profile '{}' phase '{}' needs {} context, but the caller did not supply its exact selection",
                    profile.id,
                    phase.id,
                    source.as_str()
                )));
            }
            selected.extend(source_atoms.iter());
        }
        ("context", selected, sources)
    };
    let pieces = resolve_pieces(&selected, situation, read_file, Some(&phase.id), skipped)?;
    let estimated_tokens = pieces.iter().fold(0u32, |sum, piece| {
        sum.saturating_add(piece.estimated_tokens)
    });
    Ok(ComposedPhase {
        id: phase.id.clone(),
        kind,
        sources,
        pieces,
        estimated_tokens,
    })
}

fn select(
    atoms: &[Atom],
    situation: Situation,
    runtime: ProfileRuntime,
) -> Result<BTreeMap<&str, &Atom>, ComposeError> {
    let by_id: BTreeMap<&str, &Atom> = atoms.iter().map(|atom| (atom.id.as_str(), atom)).collect();
    let tags = tags_for(situation);
    let mut selected: BTreeMap<&str, &Atom> = BTreeMap::new();
    for atom in atoms {
        if atom.profile_only || !fits(atom, runtime) {
            continue;
        }
        if atom.situations.iter().any(|tag| tags.contains(tag)) {
            selected.insert(atom.id.as_str(), atom);
        }
    }
    let mut queue: Vec<&str> = selected.keys().copied().collect();
    while let Some(id) = queue.pop() {
        let reqs = selected.get(id).and_then(|atom| atom.requires.as_ref());
        for req in reqs.into_iter().flatten() {
            if selected.contains_key(req.as_str()) {
                continue;
            }
            let Some(dep) = by_id.get(req.as_str()).copied() else {
                return Err(fail(format!(
                    "atom '{id}' requires '{req}', which is not in the graph — the {} profile cannot close.",
                    situation.as_str()
                )));
            };
            if !fits(dep, runtime) {
                return Err(fail(format!(
                    "atom '{id}' requires '{req}', but '{req}' is declared runtime={} and this compose targets runtime={} — \
                     the closure would silently thin the {} walk; fix the graph (retag '{req}' or drop the edge).",
                    dep.runtime.as_str(),
                    runtime.as_str(),
                    situation.as_str()
                )));
            }
            selected.insert(req.as_str(), dep);
            queue.push(req.as_str());
        }
    }
    Ok(selected)
}

fn tags_for(situation: Situation) -> BTreeSet<Situation> {
    match situation {
        Situation::Fresh => BTreeSet::from([Situation::Fresh]),
        Situation::Handover => BTreeSet::from([Situation::Fresh, Situation::Handover]),
        Situation::PostCompaction => {
            BTreeSet::from([Situation::PostCompaction, Situation::Handover])
        }
    }
}

fn fits(atom: &Atom, runtime: ProfileRuntime) -> bool {
    atom.runtime.as_str() == "any" || atom.runtime.as_str() == runtime.as_str()
}

fn resolve_pieces(
    atoms: &[&Atom],
    situation: Situation,
    read_file: &mut dyn FnMut(&str) -> Result<String, ReadFail>,
    phase_id: Option<&str>,
    skipped: &mut Vec<(String, String)>,
) -> Result<Vec<ComposedPiece>, ComposeError> {
    let mut pieces = Vec::new();
    for atom in atoms {
        if let Some(piece) = one_piece(atom, situation, read_file, phase_id, skipped)? {
            pieces.push(piece);
        }
    }
    Ok(pieces)
}

fn one_piece(
    atom: &Atom,
    situation: Situation,
    read_file: &mut dyn FnMut(&str) -> Result<String, ReadFail>,
    phase_id: Option<&str>,
    skipped: &mut Vec<(String, String)>,
) -> Result<Option<ComposedPiece>, ComposeError> {
    let parsed = parse_address(&atom.address).map_err(|err| fail(err.message))?;
    let file_text = match read_file(&parsed.refer) {
        Ok(text) => text,
        Err(ReadFail::Absent(_))
            if situation == Situation::PostCompaction && is_seat_recap_address(&atom.address) =>
        {
            skipped.push((atom.id.clone(), atom.address.clone()));
            return Ok(None);
        }
        Err(err) => {
            return Err(fail(format!(
                "atom '{}' ({}): source file '{}' is unreadable — {}",
                atom.id,
                atom.address,
                parsed.refer,
                err.message()
            )));
        }
    };
    let text = if parsed.header_path.is_empty() {
        file_text
    } else {
        resolve_address(&file_text, &parsed.header_path)
            .map(|section| section.text)
            .map_err(|err| {
                fail(format!(
                    "atom '{}' ({}): {}",
                    atom.id, atom.address, err.message
                ))
            })?
    };
    let source_kind = match source_kind_for_address(&atom.address) {
        Ok(kind) => kind.as_str().to_owned(),
        Err(_) => "library".to_owned(),
    };
    Ok(Some(ComposedPiece {
        atom_id: atom.id.clone(),
        address: atom.address.clone(),
        source_kind,
        order: atom.order,
        priority: atom.priority,
        estimated_tokens: estimate_tokens(&text),
        phase_id: phase_id.map(str::to_owned),
        text,
    }))
}

struct Done {
    situation: Situation,
    runtime: ProfileRuntime,
    profile_id: Option<String>,
    phases: Vec<ComposedPhase>,
    pieces: Vec<ComposedPiece>,
    skipped: Vec<(String, String)>,
    budget_tokens: Option<u32>,
}

fn finish(done: Done) -> ComposedProfile {
    let Done {
        situation,
        runtime,
        profile_id,
        phases,
        pieces,
        skipped,
        budget_tokens,
    } = done;
    let total = pieces.iter().fold(0u32, |sum, piece| {
        sum.saturating_add(piece.estimated_tokens)
    });
    ComposedProfile {
        situation,
        runtime,
        budget: budget_report(&pieces, total, budget_tokens),
        pieces,
        total_estimated_tokens: total,
        profile_id,
        phases,
        skipped,
    }
}

fn budget_report(
    pieces: &[ComposedPiece],
    total: u32,
    budget: Option<u32>,
) -> Option<BudgetReport> {
    let limit = budget?;
    if total <= limit {
        return None;
    }
    let mut ranked = pieces.to_vec();
    ranked.sort_by(|left, right| {
        drop_rank(left.priority)
            .cmp(&drop_rank(right.priority))
            .then(right.estimated_tokens.cmp(&left.estimated_tokens))
            .then(left.atom_id.cmp(&right.atom_id))
    });
    Some(BudgetReport {
        limit_tokens: limit,
        overage_tokens: total.saturating_sub(limit),
        drop_candidates: ranked
            .into_iter()
            .map(|piece| DropCandidate {
                atom_id: piece.atom_id,
                priority: piece.priority,
                estimated_tokens: piece.estimated_tokens,
            })
            .collect(),
    })
}

fn drop_rank(priority: Priority) -> u8 {
    match priority {
        Priority::Optional => 0,
        Priority::Recommended => 1,
        Priority::Core => 2,
    }
}

fn fail(message: String) -> ComposeError {
    ComposeError { message }
}
