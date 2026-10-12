//! The daemon: HTTP on a Unix socket, stub-seat launch, and pane death.

mod chat;
mod db;
mod death;
mod error;
mod events;
mod http;
mod launch;
mod query;
mod reconcile;
mod runtime;
mod serve;
mod settle;
mod state;
mod submit;
mod view;

pub use error::Error;
pub use serve::run;

#[cfg(test)]
mod flow;
