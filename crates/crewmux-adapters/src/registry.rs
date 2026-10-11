//! Map of [`RuntimeKind`](crewmux_types::RuntimeKind) to an adapter.

use std::collections::HashMap;
use std::sync::Arc;

use crewmux_types::RuntimeKind;

use crate::adapter::AgentAdapter;

/// Adapters the daemon can launch. Slice 1 registers the stub only.
#[derive(Default)]
pub struct AdapterRegistry {
    adapters: HashMap<RuntimeKind, Arc<dyn AgentAdapter>>,
}

impl AdapterRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Replace any adapter already stored for `adapter.runtime()`.
    pub fn register(&mut self, adapter: Arc<dyn AgentAdapter>) {
        self.adapters.insert(adapter.runtime(), adapter);
    }

    /// The adapter for `runtime`, when one was registered.
    #[must_use]
    pub fn get(&self, runtime: RuntimeKind) -> Option<Arc<dyn AgentAdapter>> {
        self.adapters.get(&runtime).cloned()
    }
}
