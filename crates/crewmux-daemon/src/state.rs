//! Shared daemon state. Cheap to clone into axum tasks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crewmux_adapters::AdapterRegistry;
use crewmux_messaging::{Mailboxes, QueueBook};
use crewmux_mux::Multiplexer;
use crewmux_orchestration::Engine;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::chat::ChatHub;
use crate::error::Error;
use crate::hub::EventHub;
use crate::taps::{DraftAges, KeyTaps};

/// What one process of the daemon holds.
#[derive(Clone)]
pub(crate) struct App {
    /// SQLite writer.
    pub store: Arc<crewmux_store::Store>,
    /// tmux, or [`crewmux_mux::FakeMux`] in debug builds that set `RIG_FAKE_MUX=1`.
    pub mux: Arc<dyn Multiplexer>,
    /// One mutex per rig name so concurrent `up` calls cannot insert the same seat twice.
    pub up_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Fallback when `RIG_STUB_SCRIPT` is unset. Each launch re-reads the environment.
    pub stub_script: Option<String>,
    /// Shutdown signal.
    pub cancel: CancellationToken,
    /// Milliseconds of the last HTTP request, for the idle reap.
    pub activity: Arc<AtomicU64>,
    /// Registered runtimes. Slice 1 has the stub.
    pub registry: Arc<AdapterRegistry>,
    /// `$RIG_HOME`, for transcripts and session JSONL.
    pub home: PathBuf,
    /// Live queue. SQLite is updated by the persist worker.
    pub queue: Arc<tokio::sync::Mutex<QueueBook>>,
    /// Workflow document. Committed with the queue in one store transaction.
    pub workflow: Arc<tokio::sync::Mutex<Engine>>,
    /// Inbox and outbox.
    pub mail: Arc<tokio::sync::Mutex<Mailboxes>>,
    /// Canonical `/api/events` hub.
    pub hub: Arc<EventHub>,
    /// Terminal keystrokes that tmux may not see.
    pub taps: Arc<KeyTaps>,
    /// How long a stub draft has been sitting.
    pub drafts: Arc<DraftAges>,
    /// Monotonic delivery cursor for slice 8.
    pub cursor: Arc<AtomicU64>,
    /// Rotate a transcript generation after this many bytes.
    pub transcript_cap: u64,
    /// `RIG_TRANSCRIPTS=0` turns the writer off.
    pub transcripts: bool,
    /// Background work owned by this process, including chat watch tasks.
    pub tasks: TaskTracker,
    /// Wakes chat watchers after a committed message. Not the slice 4 event bus.
    pub chat: ChatHub,
    persist_tx: mpsc::UnboundedSender<QueueWrite>,
    queue_rx: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<QueueWrite>>>>,
    /// Next snapshot write fails at this `replace_book` hook, then the hook clears.
    persist_crash: Arc<tokio::sync::Mutex<Option<&'static str>>>,
    /// In-process direct-delivery phase. `before`, `ack`, or absent.
    ///
    /// The file hook under `$RIG_HOME/direct-phase` is for the binary. Tests
    /// cannot use `std::env::set_var`, so they set this instead.
    direct_phase: Arc<tokio::sync::Mutex<Option<&'static str>>>,
    /// Crash-cart phase in the low bits and a generation in the high bits.
    ///
    /// Cancel only applies to the generation that was running when it arrived.
    cart_state: Arc<AtomicU64>,
    /// The next `restore_rig` raises the crash-cart cancel flag and still finishes.
    cancel_on_restore: Arc<AtomicBool>,
    /// In-memory context-pack library. Materialized on the first context route.
    pub context: Arc<crate::context_hold::ContextHold>,
    /// Watchdog outbox clock and test knobs.
    pub(crate) outbox: crate::outbox::OutboxCtrl,
}

const CART_IDLE: u64 = 0;
const CART_RUNNING: u64 = 1;
const CART_CANCEL: u64 = 2;

fn cart_phase(state: u64) -> u64 {
    state & 0b11
}

fn cart_generation(state: u64) -> u64 {
    state >> 2
}

fn cart_pack(generation: u64, phase: u64) -> u64 {
    (generation << 2) | phase
}

/// One queue snapshot for the writer, plus the callers waiting on that commit.
pub(crate) struct QueueWrite {
    /// Book to persist. A newer epoch replaces an older one still in the channel.
    pub book: QueueBook,
    /// Completed after the snapshot write, including a store error.
    pub acks: Vec<oneshot::Sender<Result<(), String>>>,
    /// Test hook forwarded to `replace_book` or `commit_projection`. Production writes pass `None`.
    pub crash: Option<&'static str>,
    /// Workflow document committed with `book` when present.
    pub workflow: Option<String>,
}

impl App {
    /// Build the in-memory half of the daemon. Call [`launch_workers`](Self::launch_workers) once.
    #[must_use]
    pub(crate) fn assemble(
        store: Arc<crewmux_store::Store>,
        mux: Arc<dyn Multiplexer>,
        registry: Arc<AdapterRegistry>,
        stub_script: Option<String>,
        cancel: CancellationToken,
        home: PathBuf,
    ) -> Self {
        let (persist_tx, queue_rx) = mpsc::unbounded_channel();
        let context = Arc::new(crate::context_hold::ContextHold::from_env(home.clone()));
        Self {
            store,
            mux,
            up_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            stub_script,
            cancel,
            activity: Arc::new(AtomicU64::new(now_ms())),
            registry,
            home,
            queue: Arc::new(tokio::sync::Mutex::new(QueueBook::new())),
            workflow: Arc::new(tokio::sync::Mutex::new(Engine::default())),
            mail: Arc::new(tokio::sync::Mutex::new(Mailboxes::new())),
            hub: Arc::new(EventHub::new()),
            taps: Arc::new(KeyTaps::default()),
            drafts: Arc::new(DraftAges::default()),
            cursor: Arc::new(AtomicU64::new(0)),
            transcript_cap: transcript_cap_from_env(),
            transcripts: transcripts_from_env(),
            tasks: TaskTracker::new(),
            chat: ChatHub::new(),
            persist_tx,
            queue_rx: Arc::new(tokio::sync::Mutex::new(Some(queue_rx))),
            persist_crash: Arc::new(tokio::sync::Mutex::new(None)),
            direct_phase: Arc::new(tokio::sync::Mutex::new(None)),
            cart_state: Arc::new(AtomicU64::new(cart_pack(0, CART_IDLE))),
            cancel_on_restore: Arc::new(AtomicBool::new(false)),
            context,
            outbox: crate::outbox::OutboxCtrl::default(),
        }
    }

    /// Stop the fleet after the rig that is currently restoring.
    ///
    /// Returns whether a walk was in progress. A cancel that arrives after the
    /// walk has finished does not stick to the next cart.
    pub(crate) fn request_cart_cancel(&self) -> bool {
        let current = self.cart_state.load(Ordering::SeqCst);
        let phase = cart_phase(current);
        if phase == CART_IDLE {
            return false;
        }
        if phase == CART_CANCEL {
            return true;
        }
        let next = cart_pack(cart_generation(current), CART_CANCEL);
        if self
            .cart_state
            .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return true;
        }
        let now = self.cart_state.load(Ordering::SeqCst);
        cart_generation(now) == cart_generation(current) && cart_phase(now) == CART_CANCEL
    }

    /// Whether a crash-cart cancel arrived, including during the in-flight rig.
    pub(crate) fn cart_cancel_requested(&self) -> bool {
        cart_phase(self.cart_state.load(Ordering::SeqCst)) == CART_CANCEL
    }

    /// Claim the single crash-cart walk. False when one is already in progress.
    pub(crate) fn try_begin_cart(&self) -> bool {
        loop {
            let current = self.cart_state.load(Ordering::SeqCst);
            if cart_phase(current) != CART_IDLE {
                return false;
            }
            let next = cart_pack(cart_generation(current), CART_RUNNING);
            if self
                .cart_state
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// The walk returned. The next cart starts with cancel clear.
    pub(crate) fn end_cart(&self) {
        loop {
            let current = self.cart_state.load(Ordering::SeqCst);
            if cart_phase(current) == CART_IDLE {
                return;
            }
            let next = cart_pack(cart_generation(current).wrapping_add(1), CART_IDLE);
            if self
                .cart_state
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return;
            }
        }
    }

    /// The next restore raises the crash-cart cancel flag as that rig starts.
    #[cfg(test)]
    pub(crate) fn arm_cancel_on_restore(&self) {
        self.cancel_on_restore.store(true, Ordering::SeqCst);
    }

    /// Take the one-shot "cancel during this restore" arm.
    pub(crate) fn take_cancel_on_restore(&self) -> bool {
        self.cancel_on_restore.swap(false, Ordering::SeqCst)
    }

    /// Drive the direct-delivery crash phase for in-process tests.
    #[cfg(test)]
    pub(crate) async fn set_direct_phase(&self, phase: Option<&'static str>) {
        *self.direct_phase.lock().await = phase;
    }

    /// The in-process phase, when a test set one.
    pub(crate) async fn direct_phase(&self) -> Option<&'static str> {
        *self.direct_phase.lock().await
    }

    /// Fail the next queue snapshot at the store's `before-commit` hook.
    #[cfg(test)]
    pub(crate) async fn fail_next_queue_save(&self) {
        *self.persist_crash.lock().await = Some("before-commit");
    }

    /// Start the queue writer and the transcript writer. Idempotent for the queue side.
    pub(crate) async fn launch_workers(&self) {
        if let Some(rx) = self.queue_rx.lock().await.take() {
            self.tasks
                .spawn(crate::persist::queue_writer(self.clone(), rx));
        }
        self.tasks.spawn(crate::transcripts::run(self.clone()));
        self.tasks.spawn(crate::health_watch::run(self.clone()));
        self.tasks.spawn(crate::outbox::run(self.clone()));
    }

    /// Remember the highest receipt cursor already stored. The next id is one past it.
    pub(crate) fn seed_cursor(&self, max_cursor: u64) {
        self.cursor.store(max_cursor, Ordering::Relaxed);
    }

    /// Queue the latest book for the writer. The writer keeps the newest epoch.
    ///
    /// Callers that must not return before the commit use [`commit_queue`](Self::commit_queue).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "integration tests queue a snapshot without waiting; HTTP handlers use commit_queue"
        )
    )]
    pub(crate) fn submit_queue(&self, book: QueueBook) {
        drop(self.persist_tx.send(QueueWrite {
            book,
            acks: Vec::new(),
            crash: None,
            workflow: None,
        }));
    }

    /// Persist `book` and wait until the write finishes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Message`] when the writer has stopped or the snapshot write failed.
    pub(crate) async fn commit_queue(&self, book: QueueBook) -> Result<(), Error> {
        self.commit_documented(book, None).await
    }

    /// Persist `book` and, when present, the workflow document in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Message`] when the writer has stopped or the snapshot write failed.
    pub(crate) async fn commit_documented(
        &self,
        book: QueueBook,
        workflow: Option<String>,
    ) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        let crash = self.persist_crash.lock().await.take();
        self.persist_tx
            .send(QueueWrite {
                book,
                acks: vec![tx],
                crash,
                workflow,
            })
            .map_err(|_| Error::Message("queue writer stopped".to_owned()))?;
        match rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(Error::Message(format!("queue snapshot: {err}"))),
            Err(_) => Err(Error::Message("queue writer stopped".to_owned())),
        }
    }

    /// Remember that a request is in progress so the idle reap waits.
    pub(crate) fn touch(&self) {
        self.activity.store(now_ms(), Ordering::Relaxed);
    }

    /// Serialize `up` for one rig name. The guard is held across the launch awaits.
    pub(crate) async fn lock_rig(&self, name: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let mutex = {
            let mut guard = self.up_locks.lock().await;
            Arc::clone(
                guard
                    .entry(name.to_owned())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        mutex.lock_owned().await
    }
}

fn transcript_cap_from_env() -> u64 {
    std::env::var("RIG_TRANSCRIPT_CAP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_048_576)
}

fn transcripts_from_env() -> bool {
    std::env::var("RIG_TRANSCRIPTS").ok().as_deref() != Some("0")
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
}

pub(crate) fn socket_name_from_env() -> String {
    std::env::var("RIG_TMUX_SOCKET").unwrap_or_else(|_| "rig-rs".to_owned())
}

pub(crate) fn stub_script_from_env() -> Option<String> {
    std::env::var("RIG_STUB_SCRIPT")
        .ok()
        .filter(|value| !value.is_empty())
}

pub(crate) fn stub_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("RIG_STUB_BIN") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("rig-stub-agent")))
        .unwrap_or_else(|| PathBuf::from("rig-stub-agent"))
}

pub(crate) fn idle_exceeded(activity: &AtomicU64) -> bool {
    let then = activity.load(Ordering::Relaxed);
    now_ms().saturating_sub(then) > 180_000
}

/// Map a join error from `spawn_blocking`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err passes JoinError by value and Display does not consume it"
)]
pub(crate) fn join_err(err: tokio::task::JoinError) -> Error {
    Error::Message(err.to_string())
}
