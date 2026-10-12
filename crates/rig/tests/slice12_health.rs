//! `rig health`, `rig stream`, and `rig usage` against a temp daemon.

#[expect(
    dead_code,
    reason = "shared CLI harness also serves spec and seat helpers this binary does not call"
)]
mod common;

use common::{Env, stdout};
use serde_json::json;

#[test]
fn health_json_prints_the_list_schema() {
    let env = Env::new();
    let (code, out, err) = stdout(env.rig().args(["health", "--json"]));
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(list["schema"], "openrig.health-list/v0alpha1");
    assert_eq!(list["records"].as_array().expect("records").len(), 0);

    let (code, out, err) = stdout(env.rig().args(["usage", "--json"]));
    assert_eq!(code, 0, "{err}");
    let usage: serde_json::Value = serde_json::from_str(&out).expect("usage");
    assert_eq!(usage["samples"].as_array().expect("samples").len(), 0);

    let socket = crewmux_api::daemon_socket(&env.rig_home);
    let sample =
        serde_json::to_vec(&json!({"seat": "seat@rig", "usedTokens": 99, "tokenLimit": 100}))
            .expect("sample");
    let posted = crewmux_api::Client::over_uds(&socket)
        .request("POST", "/api/usage/sample", Some(&sample))
        .expect("post");
    assert_eq!(posted.status, 200, "{}", posted.body);

    let (code, out, err) = stdout(env.rig().args(["health", "--json"]));
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(list["records"][0]["detector"], "context.pressure");
    assert_eq!(list["records"][0]["severity"], "critical");

    let (code, out, err) = stdout(env.rig().args([
        "stream", "emit", "--body", "note", "--id", "item-1", "--json",
    ]));
    assert_eq!(code, 0, "{err}");
    let item: serde_json::Value = serde_json::from_str(&out).expect("emit");
    assert_eq!(item["streamItemId"], "item-1");

    let (code, out, err) = stdout(env.rig().args(["stream", "list", "--json"]));
    assert_eq!(code, 0, "{err}");
    let listed: serde_json::Value = serde_json::from_str(&out).expect("list");
    assert_eq!(listed[0]["id"], "item-1");

    let (code, out, err) = stdout(env.rig().args(["stream", "watch", "--once"]));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("stream_emitted"), "{out}");
}
