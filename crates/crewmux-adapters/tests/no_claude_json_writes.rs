//! Launch must not touch the Claude user config or anything under `~/.claude`.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "temp-dir setup failures should fail the test"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crewmux_adapters::{AgentAdapter, ClaudeAdapter, LaunchOpts};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_types::SeatId;

#[path = "common/mod.rs"]
mod support;

#[tokio::test]
async fn launch_leaves_the_user_config_tree_unchanged() {
    let root = std::env::temp_dir().join(format!("rig-home-{}", std::process::id()));
    drop(fs::remove_dir_all(&root));
    let home = root.join("user");
    let claude = home.join(dot_claude());
    fs::create_dir_all(claude.join("projects")).expect("projects");
    let config = home.join(user_config_name());
    fs::write(&config, "{\"numStartups\":1}\n").expect("config");
    fs::write(claude.join("projects").join("keep.txt"), "stay\n").expect("keep");
    let before = snapshot(&home);
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    mux.set_capture(support::capture(&ready)).await;
    let mux: Arc<dyn Multiplexer> = mux;
    let adapter = ClaudeAdapter::new(mux, root.join("seats"), home.clone());
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    drop(
        adapter
            .launch(
                &binding,
                LaunchOpts {
                    session_name: "seat__rig".to_owned(),
                    env: BTreeMap::new(),
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
    assert_eq!(snapshot(&home), before);
    drop(fs::remove_dir_all(&root));
}

fn snapshot(home: &Path) -> BTreeMap<PathBuf, Stamp> {
    let mut out = BTreeMap::new();
    walk(home, &mut out);
    out
}

fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Stamp>) {
    let entries = fs::read_dir(dir).expect("read dir");
    for entry in entries {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(&path, out);
            continue;
        }
        let meta = fs::metadata(&path).expect("meta");
        let bytes = fs::read(&path).expect("bytes");
        out.insert(
            path,
            Stamp {
                len: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                inode: inode_of(&meta),
                checksum: checksum(&bytes),
            },
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: SystemTime,
    inode: u64,
    checksum: u64,
}

fn inode_of(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    }
    #[cfg(not(unix))]
    {
        drop(meta);
        0
    }
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn dot_claude() -> String {
    String::from(".claude")
}

fn user_config_name() -> String {
    let mut name = dot_claude();
    name.push('.');
    name.push_str("json");
    name
}
