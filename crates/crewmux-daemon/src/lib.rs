//! The daemon: HTTP on a Unix socket, stub-seat launch, and pane death.

mod ask_http;
mod composer;
mod db;
mod death;
mod deliver;
mod error;
mod events;
mod http;
mod hub;
mod inbox_http;
mod launch;
mod notify;
mod paste_check;
mod persist;
mod query;
mod queue_http;
mod reconcile;
mod runtime;
mod send_http;
mod serve;
mod settle;
mod state;
mod taps;
mod transcripts;
mod view;

pub use error::Error;
pub use serve::run;

#[cfg(test)]
mod flow;
#[cfg(test)]
mod receipt_tests;
#[cfg(test)]
mod slice4_tests;
