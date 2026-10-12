// Derived from openrig packages/daemon/src/routes/health.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Policy, checkpoints, diagnosis, and context-usage rows.

use crewmux_health::{HealthPolicy, parse_policy};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::error::Error;

/// One context sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRow {
    /// Seat session.
    pub seat: String,
    /// Tokens in use. `None` is the unknown bucket.
    pub used_tokens: Option<i64>,
    /// Window size. Never zero.
    pub token_limit: Option<i64>,
    /// Sample time, milliseconds.
    pub sampled_at_ms: i64,
}

/// Stored policy, or the default when the table has no row.
///
/// # Errors
///
/// Returns [`Error::Health`] when the stored document does not parse.
pub fn policy_load(conn: &Connection) -> Result<HealthPolicy, Error> {
    let Some(document) = policy_document(conn)? else {
        return Ok(HealthPolicy::default());
    };
    let value = serde_json::from_str::<Value>(&document)
        .map_err(|err| Error::Health(crewmux_health::Error::Policy(err.to_string())))?;
    parse_policy(&value).map_err(Error::Health)
}

/// Raw policy document, if one was stored.
///
/// # Errors
///
/// Returns [`Error`] when the row cannot be read.
pub fn policy_document(conn: &Connection) -> Result<Option<String>, Error> {
    conn.query_row(
        "SELECT document FROM health_policy WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Replace the single policy document. The caller has already validated it.
///
/// # Errors
///
/// Returns [`Error`] when the write fails.
pub fn policy_store(conn: &Connection, document: &str) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO health_policy (id, document) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET document = excluded.document",
        params![document],
    )?;
    Ok(())
}

/// Insert or replace a checkpoint body.
///
/// # Errors
///
/// Returns [`Error`] when the write fails.
pub fn checkpoint_put(conn: &Connection, id: &str, body: &str, at_ms: i64) -> Result<(), Error> {
    put(conn, "health_checkpoints", id, body, at_ms)
}

/// Checkpoint bodies, oldest first.
///
/// # Errors
///
/// Returns [`Error`] when the query fails.
pub fn checkpoint_list(conn: &Connection) -> Result<Vec<(String, String)>, Error> {
    list(conn, "health_checkpoints")
}

/// Insert or replace a diagnosis body.
///
/// # Errors
///
/// Returns [`Error`] when the write fails.
pub fn diagnosis_put(conn: &Connection, id: &str, body: &str, at_ms: i64) -> Result<(), Error> {
    put(conn, "health_diagnosis", id, body, at_ms)
}

/// Diagnosis bodies, oldest first.
///
/// # Errors
///
/// Returns [`Error`] when the query fails.
pub fn diagnosis_list(conn: &Connection) -> Result<Vec<(String, String)>, Error> {
    list(conn, "health_diagnosis")
}

/// One diagnosis body.
///
/// # Errors
///
/// Returns [`Error`] when the row cannot be read.
pub fn diagnosis_get(conn: &Connection, id: &str) -> Result<Option<String>, Error> {
    conn.query_row(
        "SELECT body FROM health_diagnosis WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Upsert one context sample. A zero limit is rejected and is not stored.
///
/// # Errors
///
/// Returns [`Error::Health`] when `token_limit` is zero, or [`Error`] when the write fails.
pub fn usage_upsert(conn: &Connection, row: &UsageRow) -> Result<(), Error> {
    if row.token_limit == Some(0) {
        return Err(Error::Health(crewmux_health::Error::Rejected(
            "token limit must be greater than zero".into(),
        )));
    }
    conn.execute(
        "INSERT INTO context_usage (seat, used_tokens, token_limit, sampled_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(seat) DO UPDATE SET
            used_tokens = excluded.used_tokens,
            token_limit = excluded.token_limit,
            sampled_at_ms = excluded.sampled_at_ms",
        params![
            row.seat,
            row.used_tokens,
            row.token_limit,
            row.sampled_at_ms
        ],
    )?;
    Ok(())
}

/// Every context sample, ordered by seat.
///
/// # Errors
///
/// Returns [`Error`] when the query fails.
pub fn usage_list(conn: &Connection) -> Result<Vec<UsageRow>, Error> {
    let mut stmt = conn.prepare(
        "SELECT seat, used_tokens, token_limit, sampled_at_ms FROM context_usage ORDER BY seat",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(UsageRow {
            seat: row.get(0)?,
            used_tokens: row.get(1)?,
            token_limit: row.get(2)?,
            sampled_at_ms: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Seat lifecycle tokens, ordered by id.
///
/// # Errors
///
/// Returns [`Error`] when the query fails.
pub fn seat_states(conn: &Connection) -> Result<Vec<String>, Error> {
    let mut stmt = conn.prepare("SELECT state FROM seats ORDER BY id")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect::<Result<Vec<String>, _>>().map_err(Into::into)
}

fn put(conn: &Connection, table: &str, id: &str, body: &str, at_ms: i64) -> Result<(), Error> {
    let sql = format!(
        "INSERT INTO {table} (id, body, created_at_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET body = excluded.body, created_at_ms = excluded.created_at_ms"
    );
    conn.execute(&sql, params![id, body, at_ms])?;
    Ok(())
}

fn list(conn: &Connection, table: &str) -> Result<Vec<(String, String)>, Error> {
    let sql = format!("SELECT id, body FROM {table} ORDER BY created_at_ms, id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
