// Derived from openrig packages/cli/src/lib/topology-trace.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Topology chain trace. This verb does not start the daemon.

use std::path::{Path, PathBuf};

use crewmux_api::rig_home;
use serde_json::{Value, json};

use crate::error::Error;
use crate::output;

pub(crate) fn trace(
    rig: &str,
    pod: Option<&str>,
    seat: Option<&str>,
    name: &str,
    json_out: bool,
) -> Result<i32, Error> {
    let topology = std::env::var_os("RIG_TOPOLOGY_ROOT")
        .map_or_else(|| rig_home().join("topology"), PathBuf::from);
    let legacy = std::env::var_os("RIG_SHARED_DOCS_ROOT")
        .map_or_else(|| rig_home().join("shared-docs"), PathBuf::from)
        .join("rigs");
    safe("rig", rig)?;
    if let Some(pod) = pod {
        safe("pod", pod)?;
    }
    if let Some(seat) = seat {
        safe("seat", seat)?;
    }
    safe("name", name)?;
    let mut levels = vec![
        level("instance", &topology.join(name), None),
        level(
            "rig",
            &topology.join("rigs").join(rig).join(name),
            Some(legacy.join(rig).join(name)),
        ),
    ];
    if let Some(pod) = pod {
        levels.push(level(
            "pod",
            &topology
                .join("rigs")
                .join(rig)
                .join("pods")
                .join(pod)
                .join(name),
            None,
        ));
    }
    if let Some(seat) = seat {
        levels.push(level(
            "seat",
            &topology
                .join("rigs")
                .join(rig)
                .join("seats")
                .join(seat)
                .join(name),
            Some(legacy.join(rig).join("seats").join(seat).join(name)),
        ));
    }
    for item in &levels {
        if let Some(advisory) = item.get("advisory").and_then(Value::as_str) {
            output::stderr_line(&format!("ADVISORY {advisory}"));
        }
    }
    if json_out {
        output::stdout_json(&json!({
            "topologyRootSource": "default",
            "name": name,
            "topologyRoot": topology.display().to_string(),
            "levels": levels,
        }));
        return Ok(0);
    }
    output::stdout_line(&format!(
        "chain \"{name}\" under topology.root={} (source: default)",
        topology.display()
    ));
    for item in &levels {
        print_level(item);
    }
    Ok(0)
}

fn level(altitude: &str, canonical: &Path, legacy: Option<PathBuf>) -> Value {
    if canonical.is_file() {
        let content = std::fs::read_to_string(canonical).unwrap_or_default();
        return json!({
            "altitude": altitude,
            "path": canonical.display().to_string(),
            "source": "topology.root",
            "resolvedPath": canonical.display().to_string(),
            "content": content,
        });
    }
    if let Some(legacy) = legacy.filter(|path| path.is_file()) {
        let content = std::fs::read_to_string(&legacy).unwrap_or_default();
        let advisory = format!(
            "legacy-topology-read: {altitude}-level file was found at the pre-convention location {} — not under topology.root. The read succeeded; migrate this file to {}.",
            legacy.display(),
            canonical.display()
        );
        return json!({
            "altitude": altitude,
            "path": canonical.display().to_string(),
            "source": "legacy",
            "resolvedPath": legacy.display().to_string(),
            "content": content,
            "advisory": advisory,
        });
    }
    json!({
        "altitude": altitude,
        "path": canonical.display().to_string(),
        "source": "absent",
        "resolvedPath": Value::Null,
        "content": Value::Null,
    })
}

fn print_level(item: &Value) {
    let altitude = item.get("altitude").and_then(Value::as_str).unwrap_or("");
    let path = item.get("path").and_then(Value::as_str).unwrap_or("");
    let source = item.get("source").and_then(Value::as_str).unwrap_or("");
    if source == "absent" {
        output::stdout_line(&format!("\n== {altitude} — absent ({path})"));
        return;
    }
    let origin = item
        .get("resolvedPath")
        .and_then(Value::as_str)
        .filter(|_| source == "legacy")
        .map(|resolved| format!(" [LEGACY: {resolved}]"))
        .unwrap_or_default();
    output::stdout_line(&format!("\n== {altitude} — {path}{origin}"));
    let content = item.get("content").and_then(Value::as_str).unwrap_or("");
    output::stdout_line(content.trim_end());
}

fn safe(field: &str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(Error::User(format!(
            "invalid {field} \"{value}\": must be a non-blank single path segment (no separators or dot-segments)"
        )));
    }
    Ok(())
}
