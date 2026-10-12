//! `rig compact --skip-map` exits 0 when `/compact` cannot be transcript-confirmed.

mod common;

use std::process::Command;

use serde_json::Value;

use common::{Env, stdout, two_stub};

fn ready(env: &Env) -> Command {
    let mut command = env.rig();
    command
        .env("RIG_FAKE_CAPTURE", "claude-ready")
        .env("RIG_SUBMIT_WAIT_MS", "80")
        .env("RIG_CONFIRM_NON_COMPACT", "1");
    command
}

#[test]
fn skip_map_exits_zero() {
    let env = Env::new();
    let work = env.home.join("work");
    // The shared harness also builds stub specs. Touch it so this binary links the helper.
    let _stub_spec = two_stub(&work);
    std::fs::create_dir_all(&work).expect("work");
    let spec = env.write_spec(
        "claude.yaml",
        &format!(
            "name: pair\nseats:\n  - name: claude\n    runtime: claude-code\n    cwd: {}\n",
            work.display()
        ),
    );
    let (code, up, err) = stdout(ready(&env).args(["up", spec.to_str().expect("utf8"), "--json"]));
    assert_eq!(code, 0, "{err} {up}");
    let up_json: Value = serde_json::from_str(up.trim()).unwrap_or(Value::Null);
    assert_eq!(
        up_json.get("status").and_then(Value::as_str),
        Some("ready"),
        "{up_json}"
    );
    let (code, out, err) = stdout(ready(&env).args(["compact", "claude", "--skip-map", "--json"]));
    assert_eq!(code, 0, "{err} {out}");
    assert!(
        !err.contains("did not confirm"),
        "stderr still reports a missed transcript: {err}"
    );
    assert!(!out.contains("occupant_generation_unavailable"), "{out}");
    let value: Value = serde_json::from_str(out.trim()).expect("json");
    assert_eq!(
        value.get("compact_sent"),
        Some(&Value::Bool(true)),
        "{value}"
    );
}
