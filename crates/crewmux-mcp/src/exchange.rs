//! One blocking daemon call, moved off the tokio worker.

use std::fmt::Write as _;
use std::time::Duration;

use crewmux_api::{Client, Response};

use crate::caller::Caller;
use crate::error::Error;

/// What one tool sends to the daemon.
pub(crate) struct Request {
    /// HTTP method.
    pub method: &'static str,
    /// Absolute path, including a query when the route has one.
    pub path: String,
    /// Raw body. `None` sends an empty body.
    pub body: Option<Vec<u8>>,
    /// Extra headers. Seat tools add the seat pair.
    pub headers: Vec<(String, String)>,
    /// Overrides the client default. `rig_start` uses 120 seconds.
    pub timeout: Option<Duration>,
}

/// Client and caller shared by every tool.
#[derive(Clone)]
pub(crate) struct Ctx {
    /// Unix-socket client. Cloned into the blocking task.
    pub api: Client,
    /// Seat or operator.
    pub caller: Caller,
}

impl Ctx {
    /// Run `request` on a blocking thread.
    ///
    /// The client is cloned because the call blocks and must own its socket.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the worker panics or the client fails.
    pub(crate) async fn exchange(&self, request: Request) -> Result<Response, Error> {
        let client = match request.timeout {
            Some(timeout) => self.api.clone().with_timeout(timeout),
            None => self.api.clone(),
        };
        let method = request.method.to_owned();
        let path = request.path;
        let body = request.body;
        let headers = request.headers;
        let joined = tokio::task::spawn_blocking(move || {
            let pairs: Vec<(&str, &str)> = headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            client.request_with_headers(&method, &path, body.as_deref(), &pairs)
        })
        .await
        .map_err(|err| Error::Message(err.to_string()))?;
        joined.map_err(Error::from)
    }

    /// `true` when the daemon socket could not be connected.
    #[must_use]
    pub(crate) fn is_down(err: &Error) -> bool {
        matches!(err, Error::Api(crewmux_api::Error::Connect { .. }))
    }
}

/// Percent-encode one path segment the way `encodeURIComponent` does for ULIDs and slashes.
#[must_use]
pub(crate) fn encode_seg(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// JSON object body.
///
/// # Errors
///
/// Returns [`Error::Message`] when the value cannot be encoded.
pub(crate) fn json_body(value: &serde_json::Value) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(|err| Error::Message(err.to_string()))
}
