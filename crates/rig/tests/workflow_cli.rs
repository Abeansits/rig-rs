//! CLI goldens for `rig workflow`. The daemon is started by the CLI itself.

mod common;

use std::path::Path;

use common::{Env, stdout, two_stub};
use serde_json::Value;

const LINEAR: &str =
    include_str!("../../crewmux-orchestration/fixtures/workflow-specs/linear-build.yaml");

fn json_out(command: &mut std::process::Command) -> (i32, Value, String) {
    let (code, out, err) = stdout(command);
    let value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
    (code, value, err)
}

fn spec(env: &Env) -> std::path::PathBuf {
    env.write_spec("linear-build.yaml", LINEAR)
}

#[test]
fn workflow_help_says_neither_advances() {
    let env = Env::new();
    let _pair = two_stub(&env.home);
    let (code, out, err) = stdout(env.rig().args(["workflow", "--help"]));
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("run runs, watch watches, neither advances"),
        "{out}"
    );
    let (code, top, err) = stdout(env.rig().args(["--help"]));
    assert_eq!(code, 0, "{err}");
    assert!(top.contains("neither advances"), "{top}");
}

#[test]
fn bad_exit_does_not_contact_the_daemon() {
    let env = Env::new();
    let (code, body, err) = json_out(env.rig().args([
        "workflow",
        "project",
        "--instance",
        "none",
        "--current-packet",
        "qitem-1",
        "--exit",
        "banana",
        "--actor-session",
        "a@b",
        "--json",
    ]));
    assert_eq!(code, 1, "{err}");
    assert_eq!(body["ok"], false);
    let fact = body["error"]["fact"].as_str().unwrap_or("");
    assert!(fact.contains("banana"), "{fact}");
    let why = body["error"]["consequence"].as_str().unwrap_or("");
    assert!(why.contains("daemon was not contacted"), "{why}");
}

#[test]
fn validate_instantiate_project_and_reads_are_json() {
    let env = Env::new();
    let path = spec(&env);
    let created = instantiate(&env, &path);
    read_surfaces(
        &env,
        created["instance"]["instanceId"].as_str().expect("id"),
    );
    let next = handoff(&env, &created, "flow-planner@acme-flow");
    let proved = handoff_packet(&env, &created, &next, "flow-builder@acme-flow");
    finish(&env, &created, &proved);
}

fn instantiate(env: &Env, path: &Path) -> Value {
    let spec_path = path.to_str().unwrap_or("");
    let (code, report, err) = json_out(
        env.rig()
            .args(["workflow", "validate", spec_path, "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(report.get("ok"), Some(&Value::Bool(true)), "{report}");
    let (code, created, err) = json_out(env.rig().args([
        "workflow",
        "instantiate",
        spec_path,
        "--root-objective",
        "ship",
        "--created-by",
        "orch@acme-flow",
        "--json",
    ]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        text(&created, "/entryOwnerSession"),
        "flow-planner@acme-flow"
    );
    assert_eq!(
        created.pointer("/instance/version"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        created.get("replayed"),
        Some(&Value::Bool(false)),
        "{created}"
    );
    assert_eq!(created.get("advisories"), Some(&serde_json::json!([])));
    created
}

fn read_surfaces(env: &Env, id: &str) {
    let (code, listed, err) = json_out(env.rig().args(["workflow", "list", "--json"]));
    assert_eq!(code, 0, "{err}");
    assert!(listed.as_array().is_some_and(|rows| rows.len() == 1));
    let (code, shown, err) = json_out(env.rig().args(["workflow", "show", id, "--json"]));
    assert_eq!(code, 0, "{err}");
    assert!(shown.get("deadline").is_some(), "{shown}");
    let (code, traced, err) = json_out(env.rig().args(["workflow", "trace", id, "--json"]));
    assert_eq!(code, 0, "{err}");
    assert!(traced.get("trail").is_some(), "{traced}");
    let (code, guide, err) = json_out(env.rig().args(["workflow", "guidance", id, "--json"]));
    assert_eq!(code, 0, "{err}");
    assert!(guide.get("lines").is_some(), "{guide}");
    let (code, first, err) = json_out(env.rig().args(["workflow", "continue", id, "--json"]));
    assert_eq!(code, 0, "{err}");
    let (code, second, err) = json_out(env.rig().args(["workflow", "continue", id, "--json"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        first.pointer("/instance/version"),
        second.pointer("/instance/version")
    );
}

fn handoff(env: &Env, created: &Value, actor: &str) -> String {
    project(
        env,
        text(created, "/instance/instanceId"),
        text(created, "/entryQitemId"),
        "handoff",
        actor,
    )
}

fn handoff_packet(env: &Env, created: &Value, packet: &str, actor: &str) -> String {
    project(
        env,
        text(created, "/instance/instanceId"),
        packet,
        "handoff",
        actor,
    )
}

fn finish(env: &Env, created: &Value, packet: &str) {
    let id = text(created, "/instance/instanceId");
    let (code, routed, err) = json_out(env.rig().args([
        "workflow",
        "route",
        id,
        "--to",
        "flow-prover@acme-flow",
        "--actor-session",
        "orch@acme-flow",
        "--json",
    ]));
    assert_eq!(code, 0, "{err} {routed}");
    let done = project(env, id, packet, "done", "flow-prover@acme-flow");
    assert_eq!(done, "");
    let (code, watched, err) = json_out(env.rig().args(["workflow", "watch", id, "--json"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        watched.get("status").and_then(Value::as_str),
        Some("completed")
    );
    let spec_path = env.home.join("linear-build.yaml");
    let (code, run, err) = json_out(env.rig().args([
        "workflow",
        "run",
        spec_path.to_str().unwrap_or(""),
        "--root-objective",
        "ship",
        "--created-by",
        "orch@acme-flow",
        "--json",
    ]));
    assert_eq!(code, 0, "{err}");
    let other = text(&run, "/instance/instanceId");
    let (code, aborted, err) = json_out(env.rig().args([
        "workflow",
        "abort",
        other,
        "--reason",
        "stop",
        "--actor-session",
        "orch@acme-flow",
        "--json",
    ]));
    assert_eq!(code, 0, "{err} {aborted}");
    assert_eq!(
        aborted.get("status").and_then(Value::as_str),
        Some("aborted")
    );
}

fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

fn project(env: &Env, id: &str, packet: &str, exit: &str, actor: &str) -> String {
    let (code, body, err) = json_out(env.rig().args([
        "workflow",
        "project",
        "--instance",
        id,
        "--current-packet",
        packet,
        "--exit",
        exit,
        "--actor-session",
        actor,
        "--json",
    ]));
    assert_eq!(code, 0, "{err} {body}");
    text(&body, "/nextQitemId").to_owned()
}
