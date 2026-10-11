//! Per-pane output rings and subscriber fan-out.
//!
//! The control reader never waits on a subscriber. Each subscriber has a bounded
//! queue larger than the 64 KiB history ring, so one full paste plus its framing
//! still fits when the reader is briefly late. A full queue marks that subscriber
//! behind and counts a [`crate::MuxEvent::Gap`]. Other subscribers keep their own queues.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use tokio::sync::mpsc::{self, error::TrySendError};

use crate::death::DeathDedupe;
use crate::types::{MuxEvent, OutputStream, PANE_RING_BYTES, PaneInfo};

pub(crate) const CHUNK: usize = 4 * 1024;
/// Four times the history ring: a 64 KiB paste is 16 chunks, and the composer
/// frame around it must not be reported as a gap.
const QUEUE: usize = (PANE_RING_BYTES / CHUNK).saturating_mul(4);

struct Sub {
    tx: mpsc::Sender<MuxEvent>,
    dropped: Arc<AtomicUsize>,
    behind: bool,
}

#[derive(Default)]
struct WindowRec {
    session: String,
    panes: Vec<String>,
}

/// Shared pane bookkeeping owned by the control task and the public API.
#[derive(Default)]
pub(crate) struct Hub {
    rings: HashMap<String, VecDeque<u8>>,
    subs: HashMap<String, Vec<Sub>>,
    windows: HashMap<String, WindowRec>,
    dedupe: DeathDedupe,
}

impl Hub {
    /// Replace the window→pane index from a `list-panes` snapshot.
    pub(crate) fn note_panes(&mut self, panes: &[PaneInfo]) {
        self.windows.clear();
        for pane in panes {
            let entry = self.windows.entry(pane.window_id.clone()).or_default();
            entry.session.clone_from(&pane.session);
            entry.panes.push(pane.pane_id.clone());
            if !pane.dead {
                self.dedupe.note_alive(&pane.pane_id);
            }
        }
    }

    /// Panes last seen in `window_id`.
    pub(crate) fn take_window(&mut self, window_id: &str) -> Option<(String, Vec<String>)> {
        self.windows
            .remove(window_id)
            .map(|rec| (rec.session, rec.panes))
    }

    /// `true` the first time this spawn generation dies.
    pub(crate) fn should_emit(&mut self, pane_id: &str) -> bool {
        self.dedupe.emit(pane_id)
    }

    /// Subscribe to one pane. The queue is a multiple of [`PANE_RING_BYTES`].
    pub(crate) fn subscribe(&mut self, pane_id: &str) -> OutputStream {
        let (tx, rx) = mpsc::channel(QUEUE);
        let dropped = Arc::new(AtomicUsize::new(0));
        if let Some(ring) = self.rings.get_mut(pane_id)
            && !ring.is_empty()
        {
            let bytes = Bytes::copy_from_slice(ring.make_contiguous());
            drop(tx.try_send(MuxEvent::Output {
                pane_id: pane_id.to_owned(),
                bytes,
            }));
        }
        self.subs.entry(pane_id.to_owned()).or_default().push(Sub {
            tx,
            dropped: Arc::clone(&dropped),
            behind: false,
        });
        OutputStream {
            pane_id: pane_id.to_owned(),
            rx,
            dropped,
        }
    }

    /// Append one chunk and fan it out without waiting.
    pub(crate) fn push_chunk(
        &mut self,
        pane_id: &str,
        chunk: &[u8],
        events: &tokio::sync::broadcast::Sender<MuxEvent>,
    ) {
        let ring = self.rings.entry(pane_id.to_owned()).or_default();
        ring.extend(chunk.iter().copied());
        while ring.len() > PANE_RING_BYTES {
            ring.pop_front();
        }
        let payload = Bytes::copy_from_slice(chunk);
        drop(events.send(MuxEvent::Output {
            pane_id: pane_id.to_owned(),
            bytes: payload.clone(),
        }));
        let Some(subs) = self.subs.get_mut(pane_id) else {
            return;
        };
        subs.retain_mut(|sub| deliver(sub, pane_id, &payload));
    }
}

fn deliver(sub: &mut Sub, pane_id: &str, payload: &Bytes) -> bool {
    if sub.behind {
        sub.dropped.fetch_add(payload.len(), Ordering::Relaxed);
        return true;
    }
    match sub.tx.try_send(MuxEvent::Output {
        pane_id: pane_id.to_owned(),
        bytes: payload.clone(),
    }) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            sub.behind = true;
            sub.dropped.fetch_add(payload.len(), Ordering::Relaxed);
            true
        }
        Err(TrySendError::Closed(_)) => false,
    }
}

/// Push `bytes` in 4 KiB chunks, yielding between chunks so a live subscriber
/// can drain while a stalled one falls behind.
pub(crate) async fn push_output(
    hub: &tokio::sync::Mutex<Hub>,
    events: &tokio::sync::broadcast::Sender<MuxEvent>,
    pane_id: &str,
    bytes: &[u8],
) {
    if bytes.is_empty() {
        return;
    }
    for chunk in bytes.chunks(CHUNK) {
        {
            let mut guard = hub.lock().await;
            guard.push_chunk(pane_id, chunk, events);
        }
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CHUNK, Hub, QUEUE, push_output};
    use crate::types::MuxEvent;

    #[tokio::test]
    async fn slow_subscriber_observes_a_gap() {
        let hub = Arc::new(tokio::sync::Mutex::new(Hub::default()));
        let (events, _keep) = tokio::sync::broadcast::channel(64);
        let mut slow = hub.lock().await.subscribe("%1");
        let mut fast = hub.lock().await.subscribe("%1");
        let total = (QUEUE + 8) * CHUNK;
        let reader = tokio::spawn(async move {
            let mut got = 0usize;
            while got < total {
                match fast.recv().await {
                    Some(MuxEvent::Output { bytes, .. }) => got += bytes.len(),
                    other => return Err(format!("{got} then {other:?}")),
                }
            }
            Ok(got)
        });
        let chunk = vec![b'a'; 4096];
        let mut sent = 0usize;
        while sent < total {
            push_output(&hub, &events, "%1", &chunk).await;
            sent += chunk.len();
        }
        let got = reader.await.expect("join").expect("fast reader");
        assert_eq!(got, total);
        let mut saw_gap = false;
        for _ in 0..(QUEUE.saturating_add(16)) {
            match tokio::time::timeout(std::time::Duration::from_secs(2), slow.recv()).await {
                Ok(Some(MuxEvent::Gap { .. })) => {
                    saw_gap = true;
                    break;
                }
                Ok(Some(_)) => {}
                other => panic!("slow subscriber stalled: {other:?}"),
            }
        }
        assert!(saw_gap);
    }
}
