//! Shared fixtures for the Claude seat tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{
    BudgetOverrides, DeliveryCheck, Error, LaunchBudget, ProbeQuery, TurnVerdict,
};
use crewmux_mux::{Capture, FakeMux, Multiplexer};

pub(crate) fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/screens/claude")
}

pub(crate) fn read(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => panic!("{}: {err}", path.display()),
    }
}

pub(crate) fn capture(text: &str) -> Capture {
    Capture::new(text.to_owned(), (0, 0))
}

pub(crate) fn fast_budget() -> LaunchBudget {
    LaunchBudget::with_overrides(BudgetOverrides {
        first_screen: Some(Duration::from_millis(40)),
        ready_after_last_gate: Some(Duration::from_millis(40)),
        inconclusive_stable: Some(Duration::from_millis(40)),
        overall: Some(Duration::from_millis(250)),
        poll: Some(Duration::from_millis(10)),
    })
}

pub(crate) fn sent_enter(calls: &[Vec<String>]) -> bool {
    calls.iter().flatten().any(|arg| arg == "Enter")
}

pub(crate) fn sent_paste(calls: &[Vec<String>]) -> bool {
    calls
        .iter()
        .flatten()
        .any(|arg| arg == "paste-buffer" || arg == "load-buffer")
}

pub(crate) struct NullCheck;

impl DeliveryCheck for NullCheck {
    fn cursor(&self, _query: &ProbeQuery) -> Result<u64, Error> {
        Ok(0)
    }

    fn turns(&self, _query: &ProbeQuery) -> Result<Vec<String>, Error> {
        Ok(Vec::new())
    }

    fn verdict(&self, _sent: &str, _stored: &str) -> TurnVerdict {
        TurnVerdict::Different
    }
}

pub(crate) fn null_check() -> Arc<dyn DeliveryCheck> {
    Arc::new(NullCheck)
}

pub(crate) fn shared(mux: &Arc<FakeMux>) -> Arc<dyn Multiplexer> {
    let cloned: Arc<FakeMux> = Arc::clone(mux);
    cloned
}

#[test]
fn helpers_link() {
    drop(fixtures());
    let _budget = fast_budget();
    assert!(!sent_enter(&[]));
    assert!(!sent_paste(&[]));
    drop(null_check());
    drop(capture("x"));
    drop(shared(&Arc::new(FakeMux::new())));
}
