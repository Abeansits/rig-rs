//! Single writer for queue snapshots. Keeps the newest epoch.

use tokio::sync::mpsc;

use crate::state::App;

pub(crate) async fn queue_writer(
    app: App,
    mut rx: mpsc::UnboundedReceiver<crewmux_messaging::QueueBook>,
) {
    loop {
        let Some(mut book) = rx.recv().await else {
            break;
        };
        if app.cancel.is_cancelled() {
            break;
        }
        while let Ok(newer) = rx.try_recv() {
            if newer.epoch() >= book.epoch() {
                book = newer;
            }
        }
        let store = std::sync::Arc::clone(&app.store);
        let saved = tokio::task::spawn_blocking(move || {
            store.write(move |conn| crewmux_store::replace_book(conn, &book, None))
        })
        .await;
        if let Err(err) = saved {
            tracing::warn!(error = %err, "queue snapshot");
        }
    }
}
