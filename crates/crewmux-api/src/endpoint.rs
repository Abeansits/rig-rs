//! Route table shared by the daemon, the fake test server, and the client.

use crate::model::{AttachInfo, SeatList, Snapshot};

/// HTTP method on the daemon socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// GET.
    Get,
}

impl Method {
    /// Token used on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
        }
    }
}

/// One typed route. Slice 1 implements the handlers; this slice defines the contract.
pub trait Endpoint {
    /// HTTP method.
    const METHOD: Method;
    /// Path template. `{seat}` is the seat id.
    const PATH: &'static str;
    /// Request body. Unit for GET.
    type Req;
    /// Response body.
    type Resp;
}

/// `GET /api/seats`.
#[derive(Debug)]
pub struct ListSeats;

impl Endpoint for ListSeats {
    const METHOD: Method = Method::Get;
    const PATH: &'static str = "/api/seats";
    type Req = ();
    type Resp = SeatList;
}

/// `GET /api/snapshot`. Composite hydrate until slice 1's reads are composed by the client.
#[derive(Debug)]
pub struct Hydrate;

impl Endpoint for Hydrate {
    const METHOD: Method = Method::Get;
    const PATH: &'static str = "/api/snapshot";
    type Req = ();
    type Resp = Snapshot;
}

/// `GET /api/seats/{seat}/attach`.
#[derive(Debug)]
pub struct AttachInfoRoute;

impl Endpoint for AttachInfoRoute {
    const METHOD: Method = Method::Get;
    const PATH: &'static str = "/api/seats/{seat}/attach";
    type Req = ();
    type Resp = AttachInfo;
}

/// `GET /api/events`. Server-sent events. This slice adds the handler.
#[derive(Debug)]
pub struct Events;

impl Endpoint for Events {
    const METHOD: Method = Method::Get;
    const PATH: &'static str = "/api/events";
    type Req = ();
    type Resp = ();
}

/// Fill `{seat}` in [`AttachInfoRoute::PATH`].
#[must_use]
pub fn attach_path(seat: &str) -> String {
    AttachInfoRoute::PATH.replace("{seat}", seat)
}

/// macOS `sun_path` is about 104 bytes. The TypeScript TUI guards at 100.
pub const MAX_SUN_PATH_BYTES: usize = 100;

/// Use `preferred` when it fits in `sun_path`. Otherwise use `fallback_dir/file_name`.
#[must_use]
pub fn fit_sun_path(
    preferred: &std::path::Path,
    fallback_dir: &std::path::Path,
    file_name: &str,
) -> std::path::PathBuf {
    if path_bytes(preferred) <= MAX_SUN_PATH_BYTES {
        preferred.to_path_buf()
    } else {
        fallback_dir.join(file_name)
    }
}

/// Byte length of a path, which is what `sun_path` counts.
#[must_use]
pub fn path_bytes(path: &std::path::Path) -> usize {
    path.as_os_str().as_encoded_bytes().len()
}

/// `$RIG_HOME/run/daemon.sock`, or `$TMPDIR/rig-rs-<uid>/daemon.sock` when that is too long.
#[must_use]
pub fn daemon_bind_path(
    rig_home: &std::path::Path,
    tmp: &std::path::Path,
    uid: u32,
) -> std::path::PathBuf {
    let preferred = crate::daemon_socket(rig_home);
    let fallback = tmp.join(format!("rig-rs-{uid}"));
    fit_sun_path(&preferred, &fallback, "daemon.sock")
}

/// `$RIG_HOME/run/tui.sock`, with the same `sun_path` fallback as the daemon socket.
#[must_use]
pub fn tui_socket_path(
    rig_home: &std::path::Path,
    tmp: &std::path::Path,
    uid: u32,
) -> std::path::PathBuf {
    let preferred = rig_home.join("run").join("tui.sock");
    let fallback = tmp.join(format!("rig-rs-{uid}"));
    fit_sun_path(&preferred, &fallback, "tui.sock")
}
