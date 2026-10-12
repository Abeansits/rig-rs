//! The daemon: HTTP on a Unix socket, stub-seat launch, and pane death.

mod ask_http;
mod chat;
mod composer;
mod db;
mod death;
mod deliver;
mod error;
mod events;
mod health_http;
mod health_query;
mod health_watch;
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
mod stream_http;
mod submit;
mod taps;
mod transcripts;
mod view;

pub use error::Error;
pub use serve::run;

#[cfg(test)]
mod flow;
#[cfg(test)]
mod health_tests;
#[cfg(test)]
mod receipt_tests;
#[cfg(test)]
mod slice4_tests;
