//! Slice 1 routes. Each one is an [`Endpoint`].

use serde::{Deserialize, Serialize};

/// A daemon route the CLI calls.
pub trait Endpoint {
    /// HTTP method.
    const METHOD: &'static str;
    /// Absolute path.
    const PATH: &'static str;
}

/// `GET /healthz`.
#[derive(Debug)]
pub struct Healthz;

impl Endpoint for Healthz {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/healthz";
}

/// `POST /api/rigs/up`. The body is the spec YAML, not JSON.
#[derive(Debug)]
pub struct CreateRigUp;

impl Endpoint for CreateRigUp {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/rigs/up";
}

/// `GET /api/seats`.
#[derive(Debug)]
pub struct ListSeats;

impl Endpoint for ListSeats {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/seats";
}

/// `POST /api/seats/capture`.
#[derive(Debug)]
pub struct CaptureSeat;

impl Endpoint for CaptureSeat {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/seats/capture";
}

/// `POST /api/seats/attach`.
#[derive(Debug)]
pub struct AttachInfo;

impl Endpoint for AttachInfo {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/seats/attach";
}

/// `POST /api/rigs/down`.
#[derive(Debug)]
pub struct RigDown;

impl Endpoint for RigDown {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/rigs/down";
}

/// Body of [`CaptureSeat`].
#[derive(Debug, Deserialize, Serialize)]
pub struct CaptureRequest {
    /// Seat name.
    pub seat: String,
    /// When set, only the last N lines are returned.
    #[serde(default)]
    pub lines: Option<u32>,
}

/// Body of [`AttachInfo`].
#[derive(Debug, Deserialize, Serialize)]
pub struct AttachRequest {
    /// Seat name.
    pub seat: String,
}

/// Body of [`RigDown`]. An absent rig stops every rig.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct DownRequest {
    /// Rig name.
    #[serde(default)]
    pub rig: Option<String>,
}
