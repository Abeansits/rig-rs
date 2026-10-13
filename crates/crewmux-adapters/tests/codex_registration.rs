//! AC9–AC10: Codex `-c` overrides and an unchanged `config.toml`.

#[path = "codex_support.rs"]
mod support;

use std::fs;
use std::sync::Arc;

use crewmux_adapters::AgentAdapter;
use crewmux_mux::FakeMux;
use crewmux_provider::{SeatEnv, codex_mcp_overrides};
use sha2::{Digest, Sha256};

use support::{LOGGED_IN, adapter, binding, home_with, opts};

#[test]
fn overrides_use_an_absolute_command_and_seat_env() {
    let env = SeatEnv {
        seat_id: "seat".to_owned(),
        seat_token: "tok".to_owned(),
        rig_home: "/tmp/rig-home".to_owned(),
    };
    let args = codex_mcp_overrides(std::path::Path::new("/usr/bin/rig"), &env);
    assert!(
        args.iter()
            .any(|arg| arg == "mcp_servers.rig.command=/usr/bin/rig")
    );
    assert!(
        args.iter()
            .any(|arg| arg == "mcp_servers.rig.args=[\"mcp\"]")
    );
    assert!(
        args.iter()
            .any(|arg| arg == "mcp_servers.rig.env.RIG_SEAT_ID=seat")
    );
    assert!(
        args.iter()
            .any(|arg| arg == "mcp_servers.rig.env.RIG_SEAT_TOKEN=tok")
    );
    assert!(
        args.iter()
            .any(|arg| arg == "mcp_servers.rig.env.RIG_HOME=/tmp/rig-home")
    );
    assert!(!args.iter().any(|arg| arg.contains("config.toml")));
}

#[tokio::test]
async fn launch_keeps_config_bytes_and_skips_claude_config() {
    let home = home_with(LOGGED_IN);
    let config = home.codex_dir.join("config.toml");
    fs::write(&config, "model = \"gpt-5.1-codex\"\n").expect("config");
    let before = fs::read(&config).expect("read");
    let checksum = Sha256::digest(&before);
    let mux = Arc::new(FakeMux::new());
    let seat = adapter(&mux, &home.bin);
    seat.launch(&binding(&home.cwd, "seat"), opts(&home, "seat"))
        .await
        .expect("launch");
    let after = fs::read(&config).expect("after");
    assert_eq!(Sha256::digest(&after).as_slice(), checksum.as_slice());
    let calls = mux.recorded().await;
    let flat: Vec<_> = calls.into_iter().flatten().collect();
    let command = flat
        .iter()
        .find(|arg| arg.starts_with("mcp_servers.rig.command="))
        .expect("command");
    let path = command.trim_start_matches("mcp_servers.rig.command=");
    assert!(path.starts_with('/'), "{command}");
    assert!(flat.iter().any(|arg| arg.contains("mcp_servers.rig.args=")));
    assert!(!flat.iter().any(|arg| arg.contains("config.toml")));
    assert!(!home.user_dir.join(".claude").exists());
    assert!(!home.user_dir.join(".claude.json").exists());
}
