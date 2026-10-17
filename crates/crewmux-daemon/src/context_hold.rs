// Derived from openrig packages/daemon/src/domain/context-packs/context-pack-library-service.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Lazy in-memory context library. Filesystem work stays off the async worker.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crewmux_api::{legacy_context_dir, workspace_context_dir};
use crewmux_orchestration::context_packs::{
    Library, PackCode, PackError, Root, SourceType, home_roots, materialize_builtin,
};
use parking_lot::Mutex;

/// Roots and the scanned index. The index is built on first use.
pub(crate) struct ContextHold {
    home: PathBuf,
    workspace: Mutex<Option<PathBuf>>,
    topology: Mutex<PathBuf>,
    slices: Mutex<PathBuf>,
    library: Mutex<Option<Library>>,
    available: AtomicBool,
}

impl ContextHold {
    /// Capture workspace and grant roots from the environment once, at startup.
    pub(crate) fn from_env(home: PathBuf) -> Self {
        let workspace = std::env::var_os("RIG_WORKSPACE").map(PathBuf::from);
        let topology = env_path("RIG_TOPOLOGY_ROOT").unwrap_or_else(|| home.join("topology"));
        let slices = env_path("RIG_SLICES_ROOT").unwrap_or_else(|| match &workspace {
            Some(root) => root.join("missions"),
            None => home.join("missions"),
        });
        Self {
            home,
            workspace: Mutex::new(workspace),
            topology: Mutex::new(topology),
            slices: Mutex::new(slices),
            library: Mutex::new(None),
            available: AtomicBool::new(true),
        }
    }

    /// Routes return 503 while this is false. Production leaves it true.
    pub(crate) fn available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    /// Test hook for the unwired-library status.
    #[cfg(test)]
    pub(crate) fn set_available(&self, on: bool) {
        self.available.store(on, Ordering::Relaxed);
    }

    /// Replace the workspace and drop the cached index.
    #[cfg(test)]
    pub(crate) fn set_workspace(&self, path: Option<PathBuf>) {
        let mut library = self.library.lock();
        *self.workspace.lock() = path;
        *library = None;
    }

    /// Replace the profile grant roots.
    #[cfg(test)]
    pub(crate) fn set_grants(&self, topology: PathBuf, slices: PathBuf) {
        *self.topology.lock() = topology;
        *self.slices.lock() = slices;
    }

    /// Topology root, slices root, and workspace root.
    pub(crate) fn grants(&self) -> (PathBuf, PathBuf, PathBuf) {
        let workspace = self
            .workspace
            .lock()
            .clone()
            .unwrap_or_else(|| self.home.clone());
        (
            self.topology.lock().clone(),
            self.slices.lock().clone(),
            workspace,
        )
    }

    /// Run `f` on the scanned library. The mutex is not held across `.await`.
    ///
    /// # Errors
    ///
    /// Returns [`PackError`] when the shipped packs cannot be materialized.
    pub(crate) fn with_lib<T>(&self, f: impl FnOnce(&mut Library) -> T) -> Result<T, PackError> {
        let mut slot = self.library.lock();
        if slot.is_none() {
            *slot = Some(self.open()?);
        }
        let Some(library) = slot.as_mut() else {
            return Err(PackError::new(
                PackCode::StoreUnavailable,
                "context pack library is unavailable",
            ));
        };
        Ok(f(library))
    }

    fn open(&self) -> Result<Library, PackError> {
        let dest = self.home.join("builtin-context-packs");
        materialize_builtin(&dest).map_err(|err| {
            PackError::new(
                PackCode::StoreUnavailable,
                format!("failed to materialize shipped context packs: {err}"),
            )
        })?;
        let mut roots = home_roots(&self.home);
        if let Some(workspace) = self.workspace.lock().clone() {
            roots.extend(workspace_roots(&workspace));
        }
        let mut library = Library::new(roots);
        drop(library.scan());
        Ok(library)
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

fn workspace_roots(workspace: &Path) -> Vec<Root> {
    let legacy = legacy_context_dir(workspace);
    let current = workspace_context_dir(workspace);
    let mut roots = vec![Root::new(legacy.clone(), SourceType::Workspace)];
    if !same_directory(&legacy, &current) {
        roots.push(Root::new(current, SourceType::Workspace));
    }
    roots
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::ContextHold;

    #[test]
    fn setters_replace_grant_roots() {
        let hold = ContextHold::from_env(PathBuf::from("/tmp/rig-context-hold"));
        hold.set_available(false);
        assert!(!hold.available());
        hold.set_available(true);
        hold.set_workspace(Some(PathBuf::from("/tmp/rig-context-ws")));
        hold.set_grants(
            PathBuf::from("/tmp/rig-context-topo"),
            PathBuf::from("/tmp/rig-context-slices"),
        );
        let (topology, slices, workspace) = hold.grants();
        assert_eq!(topology, PathBuf::from("/tmp/rig-context-topo"));
        assert_eq!(slices, PathBuf::from("/tmp/rig-context-slices"));
        assert_eq!(workspace, PathBuf::from("/tmp/rig-context-ws"));
    }
}
