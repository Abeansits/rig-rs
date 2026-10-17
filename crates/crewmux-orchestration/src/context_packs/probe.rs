// Derived from openrig packages/daemon/src/domain/context-packs/probe-eval-bridge.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Data-shaped probe bridge. It does not import an eval harness.

use crewmux_spec::context::PackManifest;
use serde_json::{Value, json};

/// One probe that could not become a case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedProbe {
    /// Atom id.
    pub atom_id: String,
    /// Why it was skipped.
    pub reason: String,
}

/// Compiled behavior cases plus the probes that were refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledProbes {
    /// Harness-shaped objects.
    pub cases: Vec<Value>,
    /// Probes without a deterministic pattern.
    pub skipped: Vec<SkippedProbe>,
}

/// Compile probed atoms into behavior-category case objects.
#[must_use]
pub fn compile_atom_probes(manifest: &PackManifest, pack_ref: &str) -> CompiledProbes {
    let mut cases = Vec::new();
    let mut skipped = Vec::new();
    for atom in &manifest.atoms {
        let Some(probe) = &atom.probe else {
            continue;
        };
        let Some(patterns) = probe
            .expected_patterns
            .as_ref()
            .filter(|items| !items.is_empty())
        else {
            skipped.push(SkippedProbe {
                atom_id: atom.id.clone(),
                reason: "probe has no expectedPatterns — the harness's deterministic door requires at least one compilable pattern; author them on the atom's probe".to_owned(),
            });
            continue;
        };
        let rubric = probe.rubric.clone().unwrap_or_else(|| probe.expect.clone());
        cases.push(json!({
            "id": format!("{pack_ref}/{}", atom.id),
            "name": probe.expect,
            "category": "behavior",
            "prompt": probe.prompt,
            "expectedPatterns": patterns,
            "rubric": rubric,
        }));
    }
    CompiledProbes { cases, skipped }
}
