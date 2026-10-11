//! Load-time transport rules. rig-rs is Unix-socket only.

use std::net::SocketAddr;
use std::path::Path;

use super::types::{PeId, Side, Transport};
use crate::error::Error;

/// Reject a transport that does not match the side. No I/O.
///
/// # Errors
///
/// [`Error::RsTcpRejected`] when rig-rs is pointed at TCP.
/// [`Error::TsRequiresTcp`] when TypeScript is pointed at a Unix socket.
pub fn bind_transport(side: &Side, transport: &Transport) -> Result<(), Error> {
    match (side, transport) {
        (Side::Rs { .. }, Transport::Tcp(addr)) => Err(Error::RsTcpRejected { addr: *addr }),
        (Side::Rs { .. }, Transport::Uds(_)) | (Side::Ts { .. }, Transport::Tcp(_)) => Ok(()),
        (Side::Ts { .. }, Transport::Uds(_)) => Err(Error::TsRequiresTcp),
    }
}

/// Parse `tcp` or `tcp:127.0.0.1:9` into the address the loader rejects.
///
/// # Errors
///
/// Returns [`Error::Message`] when the suffix is not a socket address.
pub fn tcp_target(spec: &str) -> Result<SocketAddr, Error> {
    let rest = spec.strip_prefix("tcp").unwrap_or(spec);
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    if rest.is_empty() {
        return "127.0.0.1:9"
            .parse()
            .map_err(|_| Error::Message("bad default tcp address".to_owned()));
    }
    rest.parse()
        .map_err(|_| Error::Message(format!("bad TCP address {rest}")))
}

/// Ids of `## PE-…` headings.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file cannot be read.
pub(crate) fn load_pe_ids(path: &Path) -> Result<Vec<PeId>, Error> {
    let text = std::fs::read_to_string(path)
        .map_err(|source| crate::error::io_err(format!("read {}", path.display()), source))?;
    Ok(pe_ids_in(&text))
}

/// Headings in a markdown document. A heading is a line starting with `##`.
#[must_use]
pub fn pe_ids_in(markdown: &str) -> Vec<PeId> {
    let mut ids = Vec::new();
    for line in markdown.lines() {
        let Some(rest) = line.trim().strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start_matches('#').trim_start();
        let token = rest.split_whitespace().next().unwrap_or("");
        let token = token.trim_end_matches(':');
        if let Ok(id) = PeId::parse(token)
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    ids
}

/// Confirm `expected_diff` is a heading. No-op when the case has none.
///
/// # Errors
///
/// Returns [`Error::UnknownPe`] when the id is missing, or [`Error::Io`] on read.
pub(crate) fn require_pe(pe: Option<&PeId>, exceptions: &Path) -> Result<(), Error> {
    let Some(pe) = pe else { return Ok(()) };
    let ids = load_pe_ids(exceptions)?;
    if ids.iter().any(|id| id == pe) {
        Ok(())
    } else {
        Err(Error::UnknownPe {
            pe: pe.clone(),
            file: exceptions.to_path_buf(),
        })
    }
}
