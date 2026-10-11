// Derived from openrig packages/tui/src/refresh.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Single-flight refresh with one trailing pass.

use std::pin::Pin;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

type Task = Arc<
    dyn Fn() -> Pin<Box<dyn std::future::Future<Output = Result<(), Arc<str>>> + Send>>
        + Send
        + Sync,
>;

/// Coalesces overlapping refresh calls. A call that arrives mid-flight marks the flight dirty
/// and shares its settlement. The runner then performs one trailing pass.
pub(crate) struct SingleFlight {
    task: Task,
    inner: Arc<Mutex<Flight>>,
}

struct Flight {
    active: Option<Join>,
    dirty: bool,
    next_id: u64,
}

struct Join {
    id: u64,
    done: tokio::sync::watch::Receiver<Option<Result<(), Arc<str>>>>,
}

impl SingleFlight {
    /// Bind `task`. Each overlapping window runs it once, then once more if a call arrived.
    #[must_use]
    pub(crate) fn new<F, Fut>(task: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), Arc<str>>> + Send + 'static,
    {
        Self {
            task: Arc::new(move || Box::pin(task())),
            inner: Arc::new(Mutex::new(Flight {
                active: None,
                dirty: false,
                next_id: 1,
            })),
        }
    }

    /// Start a flight or join the one already running.
    ///
    /// The returned ticket settles with that flight, including its trailing pass.
    /// `id` matches for every caller that shares the flight. The task is spawned on
    /// `tracker` and stops when `cancel` fires.
    pub(crate) fn request(&self, tracker: &TaskTracker, cancel: CancellationToken) -> Ticket {
        let mut guard = self.inner.lock();
        let joined = guard.active.as_ref().map(|active| Ticket {
            id: active.id,
            done: active.done.clone(),
        });
        if let Some(ticket) = joined {
            guard.dirty = true;
            return ticket;
        }
        let (tx, rx) = tokio::sync::watch::channel(None);
        let id = guard.next_id;
        guard.next_id = guard.next_id.saturating_add(1);
        guard.active = Some(Join {
            id,
            done: rx.clone(),
        });
        guard.dirty = false;
        drop(guard);
        let inner = Arc::clone(&self.inner);
        let task = Arc::clone(&self.task);
        tracker.spawn(async move {
            let result = tokio::select! {
                () = cancel.cancelled() => Err(Arc::from("cancelled")),
                result = drive(Arc::clone(&inner), task) => result,
            };
            {
                let mut guard = inner.lock();
                guard.active = None;
                guard.dirty = false;
            }
            if tx.send(Some(result)).is_err() {
                tracing::debug!("refresh waiter dropped");
            }
        });
        Ticket { id, done: rx }
    }
}

/// Handle returned by [`SingleFlight::request`].
pub(crate) struct Ticket {
    /// Shared by every caller of this flight.
    pub(crate) id: u64,
    done: tokio::sync::watch::Receiver<Option<Result<(), Arc<str>>>>,
}

impl Ticket {
    /// Wait until the flight, including a trailing pass, settles.
    ///
    /// # Errors
    ///
    /// Returns the task error. A dropped driver is reported as `"refresh dropped"`.
    pub(crate) async fn wait(&mut self) -> Result<(), Arc<str>> {
        loop {
            if let Some(result) = self.done.borrow().clone() {
                return result;
            }
            if self.done.changed().await.is_err() {
                return Err(Arc::from("refresh dropped"));
            }
        }
    }
}

async fn drive(inner: Arc<Mutex<Flight>>, task: Task) -> Result<(), Arc<str>> {
    loop {
        {
            let mut guard = inner.lock();
            guard.dirty = false;
        }
        task().await?;
        let guard = inner.lock();
        if !guard.dirty {
            return Ok(());
        }
    }
}
