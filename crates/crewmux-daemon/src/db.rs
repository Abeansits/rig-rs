//! Store calls off the tokio worker.

use std::sync::Arc;

use crewmux_store::{Db, Store};

use crate::error::Error;
use crate::state::join_err;

pub(crate) async fn write<T, F>(store: &Arc<Store>, job: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: for<'a> FnOnce(&'a Db<'a>) -> Result<T, crewmux_core::Error> + Send + 'static,
{
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || {
        store.write(|conn| {
            let db = Db::new(conn);
            job(&db).map_err(crewmux_store::Error::from)
        })
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

pub(crate) async fn read<T, F>(store: &Arc<Store>, job: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: for<'a> FnOnce(&'a Db<'a>) -> Result<T, crewmux_core::Error> + Send + 'static,
{
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || {
        store.read(|conn| {
            let db = Db::new(conn);
            job(&db).map_err(crewmux_store::Error::from)
        })
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}
