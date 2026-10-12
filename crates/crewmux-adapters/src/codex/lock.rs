//! Serialise Codex launches that share a `CODEX_HOME` and a working directory.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// One permit per `(CODEX_HOME, cwd)`. Drop the permit to let the next seat launch.
#[derive(Debug, Default)]
pub struct CodexLaunchLock {
    gates: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl CodexLaunchLock {
    /// An empty lock table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wait until no other seat is launching this home and directory.
    pub async fn acquire(&self, home: &Path, cwd: &Path) -> OwnedSemaphorePermit {
        let key = format!("{}|{}", home.display(), cwd.display());
        let sem = {
            let mut gates = self.gates.lock().await;
            Arc::clone(
                gates
                    .entry(key)
                    .or_insert_with(|| Arc::new(Semaphore::new(1))),
            )
        };
        acquire_open(sem).await
    }
}

async fn acquire_open(sem: Arc<Semaphore>) -> OwnedSemaphorePermit {
    let mut current = sem;
    loop {
        match Arc::clone(&current).acquire_owned().await {
            Ok(permit) => return permit,
            // The table never closes a semaphore. A replacement keeps a closed
            // gate from wedging the seat.
            Err(_) => current = Arc::new(Semaphore::new(1)),
        }
    }
}
