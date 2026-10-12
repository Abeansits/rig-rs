//! Per-rig chat routes. Watch is a temporary SSE stream, separate from `/api/events`.

mod http;
mod hub;
mod watch;

#[cfg(test)]
mod route_limits;
#[cfg(test)]
mod route_support;
#[cfg(test)]
mod route_tests;

pub(crate) use http::routes;
pub(crate) use hub::ChatHub;
