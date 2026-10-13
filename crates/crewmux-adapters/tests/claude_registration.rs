//! AC8 and AC10: Claude launch argv and mcp.json, with no user-config writes.
#![expect(clippy::panic, reason = "a missing fixture path should fail the test")]

use std::collections::BTreeMap;
use std::sync::Arc;

use crewmux_adapters::{AgentAdapter, ClaudeAdapter, LaunchOpts};
use crewmux_mcp::TOOL_NAMES;
use crewmux_mux::FakeMux;
use crewmux_provider::{SeatEnv, claude_allowed_tools, claude_mcp_json};
use crewmux_types::SeatId;

#[path = "common/mod.rs"]
mod support;

#[test]
fn allowed_tools_are_the_fifteen_rig_names() {
    let tools = claude_allowed_tools();
    assert_eq!(tools.len(), 15);
    for (tool, name) in tools.iter().zip(TOOL_NAMES) {
        assert_eq!(tool, &format!("mcp__rig__{name}"));
    }
}

#[test]
fn claude_mcp_json_has_one_rig_server() {
    let env = SeatEnv {
        seat_id: "seat".to_owned(),
        seat_token: "tok".to_owned(),
        rig_home: "/tmp/rig-home".to_owned(),
    };
    let doc = claude_mcp_json(std::path::Path::new("/usr/bin/rig"), &env);
    assert_eq!(doc["mcpServers"]["rig"]["command"], "/usr/bin/rig");
    assert_eq!(doc["mcpServers"]["rig"]["args"][0], "mcp");
    assert_eq!(doc["mcpServers"]["rig"]["env"]["RIG_SEAT_ID"], "seat");
    assert_eq!(doc["mcpServers"]["rig"]["env"]["RIG_SEAT_TOKEN"], "tok");
    assert_eq!(doc["mcpServers"]["rig"]["env"]["RIG_HOME"], "/tmp/rig-home");
    assert_eq!(doc["mcpServers"].as_object().expect("servers").len(), 1);
}

#[tokio::test]
async fn launch_argv_allows_exactly_the_rig_tools() {
    let root = std::env::temp_dir().join(format!("rig-claude-mcp-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    let cwd = root.join("work");
    let home = root.join("home");
    std::fs::create_dir_all(&cwd).expect("cwd");
    std::fs::create_dir_all(&home).expect("home");
    let mux = Arc::new(FakeMux::new());
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    mux.set_capture(support::capture(&ready)).await;
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), home.clone());
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    let mut env = BTreeMap::new();
    env.insert("RIG_SEAT_ID".to_owned(), binding.seat.to_string());
    env.insert("RIG_SEAT_TOKEN".to_owned(), "tok".to_owned());
    env.insert("RIG_HOME".to_owned(), root.display().to_string());
    drop(
        adapter
            .launch(
                &binding,
                LaunchOpts {
                    session_name: "seat__rig".to_owned(),
                    env,
                    resume: None,
                    fork: false,
                    auto_trust: true,
                    budget: Some(support::fast_budget()),
                    claude_bin: None,
                },
            )
            .await
            .expect("launch"),
    );
    let flat: Vec<String> = mux.recorded().await.into_iter().flatten().collect();
    assert!(flat.iter().any(|arg| arg == "--mcp-config"));
    assert!(flat.iter().any(|arg| arg == "--strict-mcp-config"));
    let allowed = flat
        .windows(2)
        .find(|pair| pair[0] == "--allowedTools")
        .map(|pair| pair[1].clone())
        .expect("allowedTools");
    let names: Vec<_> = allowed.split(',').map(str::to_owned).collect();
    assert_eq!(names, claude_allowed_tools());
    let mcp =
        std::fs::read_to_string(adapter.seat_dir(&binding.seat).join("mcp.json")).expect("mcp");
    let doc: serde_json::Value = serde_json::from_str(&mcp).expect("json");
    assert_eq!(doc["mcpServers"].as_object().expect("servers").len(), 1);
    assert_eq!(doc["mcpServers"]["rig"]["args"][0], "mcp");
    assert_eq!(doc["mcpServers"]["rig"]["env"]["RIG_SEAT_TOKEN"], "tok");
    assert!(
        doc["mcpServers"]["rig"]["command"]
            .as_str()
            .is_some_and(|path| path.starts_with('/')),
        "{doc}"
    );
    assert_launch_files(
        &doc,
        &flat,
        &binding.seat.to_string(),
        &root.display().to_string(),
    );
    assert!(!home.join(".claude").exists());
    assert!(!home.join(".claude.json").exists());
    drop(std::fs::remove_dir_all(&root));
}

fn assert_launch_files(doc: &serde_json::Value, flat: &[String], seat_id: &str, rig_home: &str) {
    assert_eq!(
        json_at(doc, &["mcpServers", "rig", "env", "RIG_SEAT_ID"]),
        seat_id
    );
    assert_eq!(
        json_at(doc, &["mcpServers", "rig", "env", "RIG_SEAT_TOKEN"]),
        "tok"
    );
    assert_eq!(
        json_at(doc, &["mcpServers", "rig", "env", "RIG_HOME"]),
        rig_home
    );
    let settings = flag(flat, "--settings");
    assert!(
        settings.contains("/run/") && settings.ends_with("settings.json"),
        "{settings}"
    );
    let body = std::fs::read_to_string(&settings).unwrap_or_default();
    let hooks: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreCompact",
        "PostCompact",
        "Stop",
        "SessionEnd",
    ] {
        let command = hook_command(&hooks, event);
        assert!(command.contains("RIG_SEAT_KEY="), "{event}: {command}");
        assert!(command.contains(&format!(" hook {event}")), "{command}");
        assert!(command.contains('/'), "{command}");
    }
}

fn json_at(value: &serde_json::Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return String::new();
        };
        current = next;
    }
    current.as_str().unwrap_or("").to_owned()
}

fn hook_command(hooks: &serde_json::Value, event: &str) -> String {
    hooks
        .get("hooks")
        .and_then(|value| value.get(event))
        .and_then(|value| value.get(0))
        .and_then(|value| value.get("hooks"))
        .and_then(|value| value.get(0))
        .and_then(|value| value.get("command"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn flag(flat: &[String], name: &str) -> String {
    flat.windows(2)
        .find_map(|pair| {
            let first = pair.first()?;
            if first == name {
                pair.get(1).cloned()
            } else {
                None
            }
        })
        .unwrap_or_default()
}
