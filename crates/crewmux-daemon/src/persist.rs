//! Single writer for queue snapshots. Keeps the newest epoch.

use tokio::sync::mpsc;

use crate::state::{App, QueueWrite};

pub(crate) async fn queue_writer(app: App, mut rx: mpsc::UnboundedReceiver<QueueWrite>) {
    loop {
        let received = tokio::select! {
            biased;
            () = app.cancel.cancelled() => {
                flush_pending(&app, &mut rx).await;
                break;
            }
            write = rx.recv() => write,
        };
        let Some(mut write) = received else {
            break;
        };
        while let Ok(newer) = rx.try_recv() {
            absorb(&mut write, newer);
        }
        persist(&app, write).await;
    }
}

async fn flush_pending(app: &App, rx: &mut mpsc::UnboundedReceiver<QueueWrite>) {
    let mut pending = None;
    while let Ok(write) = rx.try_recv() {
        match pending.as_mut() {
            Some(into) => absorb(into, write),
            None => pending = Some(write),
        }
    }
    if let Some(write) = pending {
        persist(app, write).await;
    }
}

/// The book and its workflow document are one epoch. A later arrival with an
/// older epoch does not donate its document to the newer book.
fn absorb(into: &mut QueueWrite, newer: QueueWrite) {
    let QueueWrite {
        book,
        acks,
        crash,
        workflow,
    } = newer;
    if book.epoch() >= into.book.epoch() {
        into.book = book;
        into.crash = crash;
        into.workflow = workflow;
    }
    into.acks.extend(acks);
}

async fn persist(app: &App, write: QueueWrite) {
    let QueueWrite {
        book,
        acks,
        crash,
        workflow,
    } = write;
    let store = std::sync::Arc::clone(&app.store);
    let saved = tokio::task::spawn_blocking(move || {
        store.write(move |conn| match workflow.as_deref() {
            Some(doc) => crewmux_store::commit_projection(conn, &book, doc, crash),
            None => crewmux_store::replace_book(conn, &book, crash),
        })
    })
    .await;
    let result = match saved {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "queue snapshot");
            Err(err.to_string())
        }
        Err(err) => {
            tracing::warn!(error = %err, "queue snapshot");
            Err(err.to_string())
        }
    };
    for ack in acks {
        drop(ack.send(result.clone()));
    }
}

#[cfg(test)]
mod tests {
    use crewmux_core::Timestamp;
    use crewmux_messaging::{NewItem, QueueBook, SessionRef};

    use super::absorb;
    use crate::state::QueueWrite;

    fn book_at(epoch: u64) -> QueueBook {
        let mut book = QueueBook::new();
        let source = SessionRef::new("alice@rig").expect("session");
        let destination = SessionRef::new("bob@rig").expect("session");
        for index in 0..epoch {
            book.create(
                NewItem {
                    id: None,
                    source: source.clone(),
                    destination: destination.clone(),
                    body: index.to_string(),
                    priority: 0,
                    tier: None,
                    tags: Vec::new(),
                    handed_off_from: None,
                    last_nudge: None,
                    explicit_human: false,
                    fingerprint: String::new(),
                },
                Timestamp::from_millis(0),
            )
            .expect("create");
        }
        book
    }

    fn write(epoch: u64, doc: Option<&str>) -> QueueWrite {
        QueueWrite {
            book: book_at(epoch),
            acks: Vec::new(),
            crash: None,
            workflow: doc.map(str::to_owned),
        }
    }

    #[test]
    fn out_of_order_commit_keeps_the_newer_epoch_pair() {
        let mut held = write(2, Some("doc-2"));
        absorb(&mut held, write(1, Some("doc-1")));
        assert_eq!(held.book.epoch(), 2);
        assert_eq!(held.workflow.as_deref(), Some("doc-2"));

        let mut held = write(1, Some("doc-1"));
        absorb(&mut held, write(2, Some("doc-2")));
        assert_eq!(held.book.epoch(), 2);
        assert_eq!(held.workflow.as_deref(), Some("doc-2"));

        let mut held = write(2, Some("doc-2"));
        absorb(&mut held, write(3, None));
        assert_eq!(held.book.epoch(), 3);
        assert!(held.workflow.is_none());
    }
}
