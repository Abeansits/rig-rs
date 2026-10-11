//! Bounded backlog for `GET /api/events`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_api::{StreamEvent, StreamKind, StreamPayload};
use tokio::sync::{Mutex, broadcast};

/// How many events a reconnecting client can still catch.
const BACKLOG: usize = 128;

/// What a client missed.
pub(crate) enum Missed {
    /// `last` is the newest id, or the log is empty.
    CaughtUp,
    /// Every event with an id greater than `last` is still buffered.
    Replay(Vec<StreamEvent>),
    /// The backlog moved past `last`. The vec is what remains, after a gap.
    Gap(Vec<StreamEvent>),
}

/// In-memory fan-out. The ring is the resume source.
pub(crate) struct EventHub {
    next: AtomicU64,
    backlog: Mutex<VecDeque<StreamEvent>>,
    live: broadcast::Sender<StreamEvent>,
}

impl EventHub {
    pub(crate) fn new() -> Self {
        let (live, _) = broadcast::channel(256);
        Self {
            next: AtomicU64::new(0),
            backlog: Mutex::new(VecDeque::new()),
            live,
        }
    }

    pub(crate) async fn publish(
        &self,
        kind: StreamKind,
        payload: Option<StreamPayload>,
    ) -> StreamEvent {
        let id = self.next.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        let event = StreamEvent { id, kind, payload };
        {
            let mut queue = self.backlog.lock().await;
            queue.push_back(event.clone());
            while queue.len() > BACKLOG {
                queue.pop_front();
            }
        }
        drop(self.live.send(event.clone()));
        event
    }

    pub(crate) async fn missed(&self, last: Option<u64>) -> Missed {
        let queue = self.backlog.lock().await;
        let Some(last) = last else {
            return Missed::Replay(queue.iter().cloned().collect());
        };
        let Some(oldest) = queue.front().map(|event| event.id) else {
            return Missed::CaughtUp;
        };
        let newest = queue.back().map_or(oldest, |event| event.id);
        if last >= newest {
            return Missed::CaughtUp;
        }
        if last.saturating_add(1) < oldest {
            return Missed::Gap(queue.iter().cloned().collect());
        }
        Missed::Replay(
            queue
                .iter()
                .filter(|event| event.id > last)
                .cloned()
                .collect(),
        )
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.live.subscribe()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}
