//! Built-in workflow specs. Bytes are the `OpenRig` v0.6.9 fixtures.

use crewmux_core::Timestamp;
use sha2::{Digest, Sha256};

use crate::model::{CachedSpec, Engine};
use crate::time::hex_encode;

const SPECS: &[(&str, &str)] = &[
    (
        "basic-loop.yaml",
        include_str!("../fixtures/workflow-specs/basic-loop.yaml"),
    ),
    (
        "branched-remediation.yaml",
        include_str!("../fixtures/workflow-specs/branched-remediation.yaml"),
    ),
    (
        "conveyor.yaml",
        include_str!("../fixtures/workflow-specs/conveyor.yaml"),
    ),
    (
        "factory-rsi.yaml",
        include_str!("../fixtures/workflow-specs/factory-rsi.yaml"),
    ),
    (
        "gated-release.yaml",
        include_str!("../fixtures/workflow-specs/gated-release.yaml"),
    ),
    (
        "linear-build.yaml",
        include_str!("../fixtures/workflow-specs/linear-build.yaml"),
    ),
    (
        "workflow-authored-wait.yaml",
        include_str!("../fixtures/workflow-specs/workflow-authored-wait.yaml"),
    ),
];

/// Install built-in specs that are not already cached.
pub fn install(engine: &mut Engine, now: Timestamp) {
    for (file, yaml) in SPECS {
        let Ok(spec) = crewmux_spec::parse_workflow(yaml, file) else {
            continue;
        };
        let key = Engine::spec_key(&spec.id, &spec.version_label);
        if engine.specs.contains_key(&key) {
            continue;
        }
        engine.specs.insert(
            key,
            CachedSpec {
                spec_id: crewmux_types::WorkflowInstanceId::generate().to_string(),
                name: spec.id,
                version: spec.version_label,
                yaml: (*yaml).to_owned(),
                source_path: format!("builtin:{file}"),
                source_hash: hex_encode(Sha256::digest(yaml.as_bytes()).as_slice()),
                cached_at: now,
                built_in: true,
            },
        );
    }
}
