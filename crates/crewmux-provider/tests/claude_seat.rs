//! Per-seat Claude files and launch flags. Setup must not write user scope.

use crewmux_provider::{ClaudeSeatFiles, ClaudeSeatSettings, McpServerEntry};

#[test]
fn launch_flags_point_at_the_seat_directory() {
    let dir = tempfile::tempdir().unwrap();
    let seat = dir.path().join("seats").join("alpha");
    let files = ClaudeSeatFiles::new(&seat);
    let mcp = files
        .write_mcp(&[McpServerEntry {
            name: "rig".to_owned(),
            command: "rig".to_owned(),
            args: vec!["mcp".to_owned()],
        }])
        .unwrap();
    let settings = files
        .write_settings(&ClaudeSeatSettings {
            permission_mode: Some("default".to_owned()),
            hooks: serde_json::json!({}),
        })
        .unwrap();
    let flags: Vec<String> = files
        .launch_flags()
        .into_iter()
        .map(|flag| flag.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        flags,
        vec![
            "--mcp-config".to_owned(),
            mcp.display().to_string(),
            "--strict-mcp-config".to_owned(),
            "--settings".to_owned(),
            settings.display().to_string(),
        ]
    );
    let mcp_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp).unwrap()).unwrap();
    assert_eq!(mcp_json["mcpServers"]["rig"]["command"], "rig");
    assert!(seat.starts_with(dir.path()));
}
