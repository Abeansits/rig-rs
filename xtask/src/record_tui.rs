//! Record `OpenRig` v0.6.9 TUI hit maps into the committed corpus.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::diff::{find_node, locate_openrig, workspace_root};
use crate::error::{Error, io_err};

/// Run `xtask/scripts/record-tui-hitmaps.mjs` against the pinned `OpenRig` tree.
///
/// # Errors
///
/// Returns a harness error when Node, tsx, or the `OpenRig` checkout is missing,
/// or when the recorder exits non-zero.
pub fn record(openrig: Option<PathBuf>, out: Option<PathBuf>) -> Result<(), Error> {
    let openrig = match openrig {
        Some(path) => path,
        None => locate_openrig()?,
    };
    let out = out
        .unwrap_or_else(|| workspace_root().join("crates/crewmux-tui/tests/fixtures/tui-hitmap"));
    let script = workspace_root().join("xtask/scripts/record-tui-hitmaps.mjs");
    let tsx = openrig.join("node_modules/tsx/dist/esm/index.mjs");
    if !tsx.is_file() {
        return Err(Error::Message(format!(
            "tsx is not installed in {}; npm install tsx in the OpenRig v0.6.9 checkout",
            openrig.display()
        )));
    }
    let node = find_node()?;
    let mut command = Command::new(node);
    command
        .arg("--import")
        .arg(&tsx)
        .arg(&script)
        .arg(&openrig)
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|source| io_err("spawn tui hit-map recorder", source))?;
    let output = child
        .wait_with_output()
        .map_err(|source| io_err("wait for tui hit-map recorder", source))?;
    if !output.status.success() {
        return Err(Error::Message(format!(
            "tui hit-map recorder failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}
