//! Per-seat `-c` MCP registration does not edit `config.toml`.

#[path = "codex_support.rs"]
mod support;

use std::fs;
use std::sync::Arc;

use crewmux_adapters::AgentAdapter;
use crewmux_adapters::codex::{CodexLaunchPlan, codex_launch_argv};
use crewmux_mux::FakeMux;
use crewmux_types::{CodexApproval, ThreadId};

use support::{LOGGED_IN, adapter, binding, calls_include, home_with, opts};

#[test]
fn mcp_override_uses_dash_c_and_never_passes_never() {
    let plan = CodexLaunchPlan {
        bin: std::path::PathBuf::from("codex"),
        approval: CodexApproval::Never,
        resume: ThreadId::new("thread-a").ok(),
        mcp_env: std::collections::BTreeMap::from([("RIG_SEAT_ID".to_owned(), "s".to_owned())]),
    };
    let argv = codex_launch_argv(&plan);
    assert!(argv.iter().any(|arg| arg == "-c"));
    assert!(
        argv.iter()
            .any(|arg| arg.contains("mcp_servers.rig.command="))
    );
    assert!(argv.iter().any(|arg| arg.contains("mcp_servers.rig.args=")));
    assert!(
        argv.iter()
            .any(|arg| arg.contains("mcp_servers.rig.env.RIG_SEAT_ID="))
    );
    assert!(
        argv.windows(2)
            .any(|pair| pair.first().map(String::as_str) == Some("-a")
                && pair.get(1).map(String::as_str) == Some("on-request"))
    );
    assert!(!argv.iter().any(|arg| arg == "never"));
    assert!(!argv.iter().any(|arg| arg.contains("config.toml")));
    let resume = argv.iter().position(|arg| arg == "resume").expect("resume");
    assert_eq!(argv.get(resume + 1).map(String::as_str), Some("thread-a"));
}

#[test]
fn untrusted_approval_is_passed_through() {
    let plan = CodexLaunchPlan {
        bin: std::path::PathBuf::from("codex"),
        approval: CodexApproval::Untrusted,
        resume: None,
        mcp_env: std::collections::BTreeMap::new(),
    };
    let argv = codex_launch_argv(&plan);
    assert!(argv.windows(2).any(|pair| {
        pair.first().map(String::as_str) == Some("-a")
            && pair.get(1).map(String::as_str) == Some("untrusted")
    }));
}

#[tokio::test]
async fn launch_leaves_config_toml_bytes_unchanged() {
    let home = home_with(LOGGED_IN);
    let config = home.codex_dir.join("config.toml");
    fs::write(&config, "model = \"gpt-5.1-codex\"\n").expect("config");
    let before = fs::read(&config).expect("read");
    let mux = Arc::new(FakeMux::new());
    let seat = adapter(&mux, &home.bin);
    let mut binding = binding(&home.cwd, "seat");
    binding.runtime_opts = serde_json::json!({"approval": "never"});
    seat.launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    assert_eq!(fs::read(&config).expect("after"), before);
    let calls = mux.recorded().await;
    assert!(
        calls_include(&calls, "-c")
            || calls.iter().any(|call| call
                .iter()
                .any(|arg| arg.contains("mcp_servers.rig.command")))
    );
    assert!(
        !calls
            .iter()
            .flatten()
            .any(|arg| arg.contains("config.toml"))
    );
    assert!(!calls.iter().flatten().any(|arg| arg == "never"));
}
