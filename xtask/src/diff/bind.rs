//! Load-time transport rules. rig-rs is Unix-socket only.

use std::net::SocketAddr;
use std::path::Path;

use super::types::{ObsField, PeId, Side, Transport};
use crate::error::{Error, io_err};

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

/// Fields named by the `- **Covers:**` line under `pe`.
///
/// # Errors
///
/// Returns [`Error::Message`] when the heading is missing, the line is missing,
/// or a field name is not an observation field.
pub fn covers_for(markdown: &str, pe: &PeId) -> Result<Vec<ObsField>, Error> {
    let Some(lines) = section_lines(markdown, pe) else {
        return Err(Error::Message(format!("{pe} is not a heading")));
    };
    let mut declared = None;
    for line in lines {
        let Some(rest) = covers_payload(line) else {
            continue;
        };
        if declared.is_some() {
            return Err(Error::Message(format!(
                "{pe} declares covers more than once"
            )));
        }
        declared = Some(parse_cover_list(rest)?);
    }
    let fields = declared.ok_or_else(|| {
        Error::Message(format!(
            "{pe} does not declare which fields it covers; add `- **Covers:**` under its heading"
        ))
    })?;
    if fields.is_empty() {
        return Err(Error::Message(format!("{pe} covers no fields")));
    }
    Ok(fields)
}

/// Confirm `expected_diff` is a heading and return the fields it covers.
///
/// Returns an empty list when the case has no exception.
///
/// # Errors
///
/// Returns [`Error::UnknownPe`] when the id is missing, [`Error::Message`] when
/// the heading does not declare covered fields, or [`Error::Io`] on read.
pub(crate) fn require_pe(pe: Option<&PeId>, exceptions: &Path) -> Result<Vec<ObsField>, Error> {
    let Some(pe) = pe else {
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(exceptions)
        .map_err(|source| io_err(format!("read {}", exceptions.display()), source))?;
    if !pe_ids_in(&text).iter().any(|id| id == pe) {
        return Err(Error::UnknownPe {
            pe: pe.clone(),
            file: exceptions.to_path_buf(),
        });
    }
    covers_for(&text, pe)
}

fn section_lines<'a>(markdown: &'a str, pe: &PeId) -> Option<Vec<&'a str>> {
    let mut collecting = false;
    let mut lines = Vec::new();
    for line in markdown.lines() {
        if let Some(token) = pe_heading_token(line) {
            if collecting {
                return Some(lines);
            }
            if token == pe.as_str() {
                collecting = true;
            }
            continue;
        }
        if collecting {
            lines.push(line);
        }
    }
    collecting.then_some(lines)
}

fn pe_heading_token(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix('#')?;
    if !rest.starts_with('#') {
        return None;
    }
    let rest = rest.trim_start_matches('#').trim_start();
    let token = rest.split_whitespace().next().unwrap_or("");
    let token = token.trim_end_matches(':');
    if token.is_empty() { None } else { Some(token) }
}

fn covers_payload(line: &str) -> Option<&str> {
    let trimmed = line
        .trim()
        .strip_prefix('-')
        .map_or(line.trim(), str::trim_start);
    trimmed
        .strip_prefix("**Covers:**")
        .or_else(|| trimmed.strip_prefix("covers:"))
        .map(str::trim)
}

fn parse_cover_list(text: &str) -> Result<Vec<ObsField>, Error> {
    let mut fields = Vec::new();
    for part in text.split(',') {
        let token = part.trim().trim_matches('`');
        if token.is_empty() {
            continue;
        }
        let field = ObsField::parse(token)?;
        if !fields.contains(&field) {
            fields.push(field);
        }
    }
    Ok(fields)
}
