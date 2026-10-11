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

fn absorb(into: &mut QueueWrite, newer: QueueWrite) {
    if newer.book.epoch() >= into.book.epoch() {
        into.book = newer.book;
    }
    into.acks.extend(newer.acks);
}

async fn persist(app: &App, write: QueueWrite) {
    let QueueWrite { book, acks } = write;
    let store = std::sync::Arc::clone(&app.store);
    let saved = tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::replace_book(conn, &book, None))
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
