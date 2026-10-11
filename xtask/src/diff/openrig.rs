//! Locate the pinned TypeScript `OpenRig` v0.6.9 tree.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use super::types::{OPENRIG_COMMIT, OPENRIG_TAG};
use crate::error::{Error, io_err};

/// Resolve the checkout the TS side runs from.
///
/// `OPENRIG_DIR` wins. Otherwise `~/.cache/rig-rs/openrig-v0.6.9` is tried.
/// A missing tree is [`Error::TsMissing`], never a skipped case.
///
/// # Errors
///
/// Returns [`Error::TsMissing`] with install instructions, or [`Error::Io`] on a bad read.
pub fn locate_openrig() -> Result<PathBuf, Error> {
    if let Some(dir) = std::env::var_os("OPENRIG_DIR") {
        let path = PathBuf::from(dir);
        return verify(&path);
    }
    if let Some(cache) = cache_candidate()
        && cache.is_dir()
    {
        return verify(&cache);
    }
    Err(Error::TsMissing {
        detail: install_instructions("no checkout was found"),
    })
}

fn verify(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_dir() {
        return Err(Error::TsMissing {
            detail: install_instructions(&format!("{} is not a directory", path.display())),
        });
    }
    let manifest = path.join("package.json");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|source| io_err(format!("read {}", manifest.display()), source))?;
    let json: Value = serde_json::from_str(&text).map_err(|source| Error::Json {
        context: format!("parse {}", manifest.display()),
        source,
    })?;
    let version = json.get("version").and_then(Value::as_str).unwrap_or("");
    if version != "0.6.9" {
        return Err(Error::TsMissing {
            detail: install_instructions(&format!(
                "{} reports version {version}, expected 0.6.9",
                manifest.display()
            )),
        });
    }
    if path.join(".git").exists() {
        match git_head(path) {
            Some(head) if head.starts_with(OPENRIG_COMMIT) || OPENRIG_COMMIT.starts_with(&head) => {
            }
            Some(head) => {
                return Err(Error::TsMissing {
                    detail: install_instructions(&format!(
                        "git HEAD {head} is not {OPENRIG_TAG} ({OPENRIG_COMMIT})"
                    )),
                });
            }
            None => {}
        }
    }
    Ok(path.to_path_buf())
}

fn git_head(path: &Path) -> Option<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command.spawn().ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn cache_candidate() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache/rig-rs/openrig-v0.6.9"))
}

/// Text included in [`Error::TsMissing`].
#[must_use]
pub fn install_instructions(reason: &str) -> String {
    format!(
        "{reason}\n\
The diff harness will not skip a missing TypeScript tree.\n\
Node >= 22 is required (OpenRig v0.6.9 engines: ^22 || ^24).\n\
Install OpenRig {OPENRIG_TAG} ({OPENRIG_COMMIT}) and build the CLI:\n\
  git clone --depth 1 --branch {OPENRIG_TAG} https://github.com/mvschwarz/openrig.git \"$HOME/.cache/rig-rs/openrig-v0.6.9\"\n\
  cd \"$HOME/.cache/rig-rs/openrig-v0.6.9\" && npm ci && npm run build -w @openrig/daemon && npm run build -w @openrig/cli\n\
Then re-run with OPENRIG_DIR set to that checkout, or leave it at the cache path above.\n\
`xtask diff --self` compares rig-rs to itself and does not need Node."
    )
}

/// `node` binary on `PATH`.
///
/// # Errors
///
/// Returns [`Error::TsMissing`] when `node` cannot be found.
pub fn find_node() -> Result<PathBuf, Error> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("node");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(Error::TsMissing {
        detail: install_instructions("node is not on PATH; Node >= 22 is required"),
    })
}
