//! The daemon: HTTP on a Unix socket, stub-seat launch, and pane death.

mod ask_http;
mod cart;
mod chat;
mod check_run;
mod checkpoint;
mod composer;
mod db;
mod death;
mod deliver;
mod direct;
mod durable_http;
mod error;
mod events;
mod health_http;
mod health_query;
mod health_watch;
mod http;
mod hub;
mod inbox_http;
mod launch;
mod lineage;
mod lineage_held;
mod notify;
mod paste_check;
mod persist;
mod query;
mod queue_http;
mod reconcile;
mod restore;
mod resume;
mod runtime;
mod send_http;
mod serve;
mod settle;
mod snapshot;
mod state;
mod stream_http;
mod submit;
mod taps;
mod transcripts;
mod turns;
mod view;
mod workflow;
mod workflow_http;

pub use error::Error;
pub use serve::run;

#[cfg(test)]
mod flow;
#[cfg(test)]
mod health_tests;
#[cfg(all(test, feature = "tmux-it"))]
mod lineage_it;
#[cfg(test)]
mod receipt_tests;
#[cfg(test)]
mod reply_hint_tests;
#[cfg(test)]
mod send_tests;
#[cfg(test)]
mod slice4_tests;
#[cfg(test)]
mod slice8_delivery_tests;
#[cfg(test)]
mod slice8_harness;
#[cfg(test)]
mod slice8_restore_tests;
#[cfg(test)]
mod workflow_tests;
