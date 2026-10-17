// Derived from openrig packages/daemon/src/domain/context-packs/seat-recap-store.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `POST /api/context-packs/recap` writes a seat recap under the topology root.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crewmux_orchestration::context_packs::{ContractFinding, write_seat_recap};
use crewmux_spec::context::is_safe_segment;
use serde_json::{Value, json};

use crate::context_http::RouteError;

pub(crate) fn recap_route(topology: &Path, body: &[u8]) -> Result<Value, RouteError> {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let rig = value.get("rig").and_then(Value::as_str).unwrap_or("");
    let seat = value.get("seat").and_then(Value::as_str).unwrap_or("");
    let Some(content) = value.get("content").and_then(Value::as_str) else {
        return Err(RouteError::named(
            400,
            "invalid_recap_request",
            "body must include { rig, seat, content }",
        ));
    };
    if !is_safe_segment(rig) || !is_safe_segment(seat) {
        return Err(RouteError::named(
            400,
            "unsafe_topology_segment",
            "unsafe rig or seat segment — topology addresses require one bounded path segment",
        ));
    }
    let rigs = topology.join("rigs");
    if let Some(message) = symlink_segment(&rigs).or_else(|| symlink_segment(&rigs.join(rig))) {
        return Err(RouteError::named(400, "unsafe_ref_namespace", message));
    }
    let rig_dir = rigs.join(rig);
    if !rig_dir.is_dir() {
        return Err(RouteError::named(
            404,
            "rig_not_found",
            format!(
                "topology rig '{rig}' does not exist under {}",
                topology.display()
            ),
        ));
    }
    let seats = rig_dir.join("seats");
    if let Some(message) = symlink_segment(&seats) {
        return Err(RouteError::named(400, "unsafe_ref_namespace", message));
    }
    let seat_dir = seats.join(seat);
    if let Some(message) = symlink_segment(&seat_dir) {
        return Err(RouteError::named(400, "unsafe_ref_namespace", message));
    }
    let now_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0,
    };
    let now_ms = u64::try_from(now_ms).unwrap_or(u64::MAX);
    let findings = write_seat_recap(&seat_dir, content, now_ms)
        .map_err(|err| RouteError::named(400, "recap_unaddressable", err.message))?;
    Ok(json!({
        "path": seat_dir.join("RECAP.md").display().to_string(),
        "findings": findings.iter().map(finding_json).collect::<Vec<_>>(),
    }))
}

fn finding_json(finding: &ContractFinding) -> Value {
    match finding {
        ContractFinding::NoDecisionsSection => json!({"kind": "no-decisions-section"}),
        ContractFinding::NonstandardUnverified { line } => {
            json!({"kind": "nonstandard-unverified", "line": line})
        }
    }
}

fn symlink_segment(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        Some(format!(
            "refusing to write through symlink '{}'",
            path.display()
        ))
    } else {
        None
    }
}
