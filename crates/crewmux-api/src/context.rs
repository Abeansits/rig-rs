// Derived from openrig packages/daemon/src/routes/context-packs.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Context-pack paths and the routes the CLI calls.
//!
//! Workspace packs live at `<workspace>/<STATE_DIR_NAME>/context-packs`. `rig setup`
//! adds a `.openrig` symlink so the v0.6.9 path still resolves.

use std::io;
use std::path::{Path, PathBuf};

use crate::endpoint::Endpoint;

/// Directory name under a workspace and under `$HOME` for `RIG_HOME`.
pub const STATE_DIR_NAME: &str = ".rig-rs";

/// `<workspace>/.rig-rs/context-packs`.
#[must_use]
pub fn workspace_context_dir(workspace: &Path) -> PathBuf {
    workspace.join(STATE_DIR_NAME).join("context-packs")
}

/// `<workspace>/.openrig/context-packs`, the v0.6.9 path.
#[must_use]
pub fn legacy_context_dir(workspace: &Path) -> PathBuf {
    workspace.join(".openrig").join("context-packs")
}

/// Point `workspace/.openrig` at [`STATE_DIR_NAME`] when `.openrig` is absent.
///
/// An existing file, directory, or symlink is left alone. The link target is
/// the relative name `.rig-rs`, so the workspace can move as a tree.
///
/// # Errors
///
/// Returns an error when the symlink cannot be created.
pub fn ensure_openrig_compat(workspace: &Path) -> io::Result<bool> {
    let link = workspace.join(".openrig");
    match std::fs::symlink_metadata(&link) {
        Ok(_) => Ok(false),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            std::os::unix::fs::symlink(STATE_DIR_NAME, &link)?;
            Ok(true)
        }
        Err(err) => Err(err),
    }
}

/// `GET /api/context-packs/library`.
#[derive(Debug)]
pub struct ContextLibrary;

impl Endpoint for ContextLibrary {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/context-packs/library";
}

/// `POST /api/context-packs/library/sync`.
#[derive(Debug)]
pub struct ContextSync;

impl Endpoint for ContextSync {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/context-packs/library/sync";
}

/// `POST /api/context-packs/library/compose`.
#[derive(Debug)]
pub struct ContextCompose;

impl Endpoint for ContextCompose {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/context-packs/library/compose";
}

/// `GET` or `DELETE /api/context-packs/library/by-ref`.
#[derive(Debug)]
pub struct ContextByRef;

impl Endpoint for ContextByRef {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/context-packs/library/by-ref";
}

/// `GET /api/context-packs/library/by-ref/preview`.
#[derive(Debug)]
pub struct ContextPreview;

impl Endpoint for ContextPreview {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/context-packs/library/by-ref/preview";
}

/// `GET /api/context-packs/library/by-ref/pieces`.
#[derive(Debug)]
pub struct ContextPieces;

impl Endpoint for ContextPieces {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/context-packs/library/by-ref/pieces";
}

/// `GET /api/context-packs/library/by-ref/profile`.
#[derive(Debug)]
pub struct ContextProfile;

impl Endpoint for ContextProfile {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/context-packs/library/by-ref/profile";
}

/// `GET /api/context-packs/library/resolve-address`.
#[derive(Debug)]
pub struct ContextAddress;

impl Endpoint for ContextAddress {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/context-packs/library/resolve-address";
}

/// `POST /api/context-packs/deliver`.
#[derive(Debug)]
pub struct ContextDeliver;

impl Endpoint for ContextDeliver {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/context-packs/deliver";
}

/// `POST /api/context-packs/recap`.
#[derive(Debug)]
pub struct ContextRecap;

impl Endpoint for ContextRecap {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/context-packs/recap";
}
