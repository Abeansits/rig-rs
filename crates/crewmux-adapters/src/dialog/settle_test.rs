//! Live-pace trust navigation. The clock does not sleep.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use async_trait::async_trait;

use crate::dialog::pace::{DialogClock, DialogPace, ManualClock};
use crate::dialog::policy::{AnswerPolicy, SeatContext};
use crate::dialog::{DialogDriver, DriveOutcome, EscalateReason, NavKey, PaneIo, VerifiedCursor};
use crate::error::Error;

const STICK_AFTER: Duration = Duration::from_secs(3);
const TRANSIENT: Duration = Duration::from_millis(50);

#[tokio::test]
async fn transient_yes_is_not_entered() {
    let no = fixture("claude-2.1.287-trust-navprobe-initial-no.txt");
    let yes = fixture("claude-2.1.287-trust-after-down-yes.txt");
    let ready = fixture("claude-2.1.287-ready-after-trust.txt");
    let clock = Arc::new(ManualClock::new());
    let pane = Arc::new(RacePane::new(Arc::clone(&clock), no, yes, ready));
    let cwd = PathBuf::from("/tmp/rig-qa1/openrig-spikes-scratch/rig-rs/w18probe032913");
    let home = PathBuf::from("/tmp/rig-qa1");
    let outcome = drive(&pane, &clock, &cwd, &home).await;
    let keys = pane.keys().await;
    let DriveOutcome::Answered { label, .. } = outcome else {
        panic!("settled yes {outcome:?} keys {keys:?}");
    };
    assert_eq!(label, "Yes, I trust this folder");
    let enters = keys.iter().filter(|key| *key == "enter").count();
    assert_eq!(enters, 1, "{keys:?}");
    assert!(
        keys.iter().filter(|key| *key == "down").count() >= 2,
        "{keys:?}"
    );
    for screen in pane.entered_on().await {
        assert!(
            !screen.contains("❯ No, exit"),
            "Enter while No was highlighted:\n{screen}"
        );
        assert!(screen.contains("❯ Yes, I trust this folder"), "{screen}");
    }
}

#[tokio::test]
async fn a_dialog_that_never_reaches_yes_sends_no_enter() {
    let no = fixture("claude-2.1.287-trust-navprobe-initial-no.txt");
    let clock = Arc::new(ManualClock::new());
    let pane = Arc::new(RacePane::stuck(Arc::clone(&clock), no));
    let cwd = PathBuf::from("/tmp/rig-qa1/openrig-spikes-scratch/rig-rs/w18probe032913");
    let home = PathBuf::from("/tmp/rig-qa1");
    let outcome = drive(&pane, &clock, &cwd, &home).await;
    let DriveOutcome::Escalated { reason, .. } = outcome else {
        panic!("expected escalate {outcome:?}");
    };
    assert_eq!(reason, EscalateReason::StepLimit);
    let keys = pane.keys().await;
    assert!(!keys.iter().any(|key| key == "enter"), "{keys:?}");
}

#[tokio::test]
async fn one_down_then_two_yes_reads_enter() {
    let no = fixture("claude-2.1.287-trust-navprobe-initial-no.txt");
    let yes = fixture("claude-2.1.287-trust-after-down-yes.txt");
    let ready = fixture("claude-2.1.287-ready-after-trust.txt");
    let pane = QueuePane::new(vec![no, yes.clone(), yes, ready]);
    let cwd = PathBuf::from("/tmp/rig-qa1/openrig-spikes-scratch/rig-rs/w18probe032913");
    let home = PathBuf::from("/tmp/rig-qa1");
    let driver = DialogDriver::default();
    let outcome = driver
        .drive(
            &pane,
            crate::claude::CLAUDE_DIALOGS,
            policy(),
            &seat(&cwd, &home),
        )
        .await
        .expect("drive");
    let DriveOutcome::Answered { .. } = outcome else {
        panic!("{outcome:?} keys {:?}", pane.keys().await);
    };
    assert_eq!(
        pane.keys().await,
        vec!["down".to_owned(), "enter".to_owned()]
    );
}

async fn drive(pane: &RacePane, clock: &Arc<ManualClock>, cwd: &Path, home: &Path) -> DriveOutcome {
    let shared = Arc::clone(clock);
    let shared: Arc<dyn crate::dialog::DialogClock> = shared;
    let driver = DialogDriver::paced(DialogPace::live(), shared);
    driver
        .drive(
            pane,
            crate::claude::CLAUDE_DIALOGS,
            policy(),
            &seat(cwd, home),
        )
        .await
        .expect("drive")
}

fn policy() -> AnswerPolicy {
    AnswerPolicy {
        auto_trust: true,
        auto_approve_rig_tools: false,
    }
}

fn seat<'a>(cwd: &'a Path, home: &'a Path) -> SeatContext<'a> {
    SeatContext { cwd, home }
}

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/screens/claude/2.1.287")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

struct RacePane {
    clock: Arc<ManualClock>,
    no: String,
    yes: String,
    ready: String,
    stuck: bool,
    downs: Mutex<Vec<Duration>>,
    keys: Mutex<Vec<String>>,
    entered: Mutex<Vec<String>>,
}

impl RacePane {
    fn new(clock: Arc<ManualClock>, no: String, yes: String, ready: String) -> Self {
        Self {
            clock,
            no,
            yes,
            ready,
            stuck: false,
            downs: Mutex::new(Vec::new()),
            keys: Mutex::new(Vec::new()),
            entered: Mutex::new(Vec::new()),
        }
    }

    fn stuck(clock: Arc<ManualClock>, no: String) -> Self {
        let mut pane = Self::new(clock, no, String::new(), String::new());
        pane.stuck = true;
        pane
    }

    async fn keys(&self) -> Vec<String> {
        self.keys.lock().await.clone()
    }

    async fn entered_on(&self) -> Vec<String> {
        self.entered.lock().await.clone()
    }

    async fn screen(&self) -> String {
        if !self.entered.lock().await.is_empty() {
            return self.ready.clone();
        }
        if self.stuck {
            return self.no.clone();
        }
        let downs = self.downs.lock().await.clone();
        let Some(at) = downs.last().copied() else {
            return self.no.clone();
        };
        let age = self.clock.elapsed().saturating_sub(at);
        if age < TRANSIENT || at >= STICK_AFTER {
            self.yes.clone()
        } else {
            self.no.clone()
        }
    }
}

#[async_trait]
impl PaneIo for RacePane {
    async fn capture_text(&self) -> Result<String, Error> {
        Ok(self.screen().await)
    }

    async fn nav(&self, key: NavKey) -> Result<(), Error> {
        let name = match key {
            NavKey::Down => "down",
            NavKey::Up => "up",
        };
        self.keys.lock().await.push(name.to_owned());
        self.downs.lock().await.push(self.clock.elapsed());
        Ok(())
    }

    async fn enter(&self, proof: VerifiedCursor) -> Result<(), Error> {
        let screen = self.screen().await;
        self.entered.lock().await.push(screen);
        self.keys.lock().await.push("enter".to_owned());
        if crate::dialog::policy::never_choose(&proof.label) {
            return Err(Error::Message("entered a deny-listed choice".to_owned()));
        }
        Ok(())
    }
}

struct QueuePane {
    frames: Mutex<VecDeque<String>>,
    last: Mutex<String>,
    keys: Mutex<Vec<String>>,
}

impl QueuePane {
    fn new(frames: Vec<String>) -> Self {
        let last = frames.first().cloned().unwrap_or_default();
        Self {
            frames: Mutex::new(VecDeque::from(frames)),
            last: Mutex::new(last),
            keys: Mutex::new(Vec::new()),
        }
    }

    async fn keys(&self) -> Vec<String> {
        self.keys.lock().await.clone()
    }
}

#[async_trait]
impl PaneIo for QueuePane {
    async fn capture_text(&self) -> Result<String, Error> {
        let next = self.frames.lock().await.pop_front();
        if let Some(next) = next {
            *self.last.lock().await = next.clone();
            return Ok(next);
        }
        Ok(self.last.lock().await.clone())
    }

    async fn nav(&self, key: NavKey) -> Result<(), Error> {
        let name = match key {
            NavKey::Down => "down",
            NavKey::Up => "up",
        };
        self.keys.lock().await.push(name.to_owned());
        Ok(())
    }

    async fn enter(&self, _proof: VerifiedCursor) -> Result<(), Error> {
        self.keys.lock().await.push("enter".to_owned());
        Ok(())
    }
}
