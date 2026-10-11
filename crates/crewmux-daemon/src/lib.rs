//! The daemon: HTTP server, event streams, composition root and supervisors.
//!
//! The only route this crate serves today is the temporary fan-out in
//! [`interim_events`]. Slice 4 replaces that module. Slice 1 mounts read routes
//! on the same socket later. Clients, including the TUI, depend on `crewmux-api`
//! and not on this crate.

pub mod interim_events;
