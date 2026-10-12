// Derived from openrig packages/daemon/src/domain/classifier-lease-manager.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Classifier leases. One active row, enforced by a partial unique index.

use crewmux_health::{ClassifierLease, LeaseState};
use crewmux_messaging::SessionRef;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use ulid::Ulid;

use crate::error::Error;

/// Clock the lease verbs share. Tests pass a fixed `now_ms`.
#[derive(Clone, Copy, Debug)]
pub struct LeaseClock {
    /// Current time, milliseconds.
    pub now_ms: i64,
    /// TTL added to a new lease or a heartbeat.
    pub ttl_ms: i64,
}

struct Row {
    id: String,
    holder: String,
    acquired_at_ms: i64,
    expires_at_ms: i64,
    last_heartbeat_ms: i64,
    state: String,
}

/// Acquire the single active lease, or return the caller's unexpired lease.
///
/// # Errors
///
/// Returns [`Error::Health`] with `lease_held` when another session holds the
/// active row, including after the TTL. `lease_evaluate` marks that row expired
/// before anyone else can acquire it. `now == expires_at` is still inside the TTL.
pub fn lease_acquire(
    conn: &Connection,
    holder: &str,
    clock: LeaseClock,
) -> Result<ClassifierLease, Error> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if let Some(existing) = gate(&tx, holder, clock.now_ms)? {
        tx.commit()?;
        return Ok(existing);
    }
    let lease = insert_active(&tx, holder, clock)?;
    tx.commit()?;
    Ok(lease)
}

/// Slide the TTL. A heartbeat past `expires_at` marks the lease expired and does not revive it.
///
/// # Errors
///
/// Returns [`Error::Health`] when the id, session, or state does not match an active lease.
pub fn lease_heartbeat(
    conn: &Connection,
    lease_id: &str,
    holder: &str,
    clock: LeaseClock,
) -> Result<ClassifierLease, Error> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let row = load_id(&tx, lease_id)?.ok_or(Error::Health(crewmux_health::Error::LeaseNotFound))?;
    if row.holder != holder {
        return Err(Error::Health(crewmux_health::Error::LeaseMismatch));
    }
    if row.state != "active" {
        return Err(Error::Health(crewmux_health::Error::LeaseNotActive));
    }
    if clock.now_ms > row.expires_at_ms {
        mark(&tx, &row.id, "expired")?;
        tx.commit()?;
        return Err(Error::Health(crewmux_health::Error::LeaseExpired));
    }
    let expires = clock.now_ms.saturating_add(clock.ttl_ms);
    tx.execute(
        "UPDATE classifier_leases SET last_heartbeat_ms = ?1, expires_at_ms = ?2 WHERE id = ?3",
        params![clock.now_ms, expires, lease_id],
    )?;
    tx.commit()?;
    lease_from(Row {
        last_heartbeat_ms: clock.now_ms,
        expires_at_ms: expires,
        ..row
    })
}

/// Mark active leases past their TTL as expired. This does not reclaim them.
///
/// # Errors
///
/// Returns [`Error`] when the update fails.
pub fn lease_evaluate(conn: &Connection, now_ms: i64) -> Result<u32, Error> {
    let changed = conn.execute(
        "UPDATE classifier_leases SET state = 'expired' WHERE state = 'active' AND expires_at_ms < ?1",
        params![now_ms],
    )?;
    u32::try_from(changed)
        .map_err(|_| Error::Health(crewmux_health::Error::Rejected("lease update".into())))
}

/// Operator reclaim. `if_dead` refuses while the TTL has not elapsed.
///
/// # Errors
///
/// Returns [`Error::Health`] when no lease is active, or when `if_dead` finds a live holder.
pub fn lease_reclaim(
    conn: &Connection,
    if_dead: bool,
    now_ms: i64,
) -> Result<ClassifierLease, Error> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let Some(row) = load_active(&tx)? else {
        return Err(Error::Health(crewmux_health::Error::NoActiveLease));
    };
    if if_dead && row.expires_at_ms >= now_ms {
        return Err(Error::Health(crewmux_health::Error::LeaseStillActive));
    }
    mark(&tx, &row.id, "reclaimed")?;
    tx.commit()?;
    lease_from(Row {
        state: "reclaimed".to_owned(),
        ..row
    })
}

/// The active lease, when one exists.
///
/// # Errors
///
/// Returns [`Error`] when the row cannot be read.
pub fn lease_active(conn: &Connection) -> Result<Option<ClassifierLease>, Error> {
    load_active(conn)?.map(lease_from).transpose()
}

fn gate(conn: &Connection, holder: &str, now_ms: i64) -> Result<Option<ClassifierLease>, Error> {
    let Some(row) = load_active(conn)? else {
        return Ok(None);
    };
    if row.holder != holder {
        return Err(Error::Health(crewmux_health::Error::LeaseHeld {
            holder: row.holder,
        }));
    }
    if row.expires_at_ms >= now_ms {
        return lease_from(row).map(Some);
    }
    mark(conn, &row.id, "expired")?;
    Ok(None)
}

fn insert_active(
    conn: &Connection,
    holder: &str,
    clock: LeaseClock,
) -> Result<ClassifierLease, Error> {
    let id = Ulid::generate().to_string();
    let expires = clock.now_ms.saturating_add(clock.ttl_ms);
    let inserted = conn.execute(
        "INSERT INTO classifier_leases (id, holder, acquired_at_ms, expires_at_ms, last_heartbeat_ms, state)
         VALUES (?1, ?2, ?3, ?4, ?3, 'active')",
        params![id, holder, clock.now_ms, expires],
    );
    if let Err(err) = inserted {
        if constraint(&err) {
            let winner = load_active(conn)?.map(|row| row.holder).unwrap_or_default();
            return Err(Error::Health(crewmux_health::Error::LeaseHeld {
                holder: winner,
            }));
        }
        return Err(err.into());
    }
    lease_from(Row {
        id,
        holder: holder.to_owned(),
        acquired_at_ms: clock.now_ms,
        expires_at_ms: expires,
        last_heartbeat_ms: clock.now_ms,
        state: "active".to_owned(),
    })
}

fn load_active(conn: &Connection) -> Result<Option<Row>, Error> {
    conn.query_row(
        "SELECT id, holder, acquired_at_ms, expires_at_ms, last_heartbeat_ms, state
         FROM classifier_leases WHERE state = 'active'",
        [],
        map_row,
    )
    .optional()
    .map_err(Into::into)
}

fn load_id(conn: &Connection, id: &str) -> Result<Option<Row>, Error> {
    conn.query_row(
        "SELECT id, holder, acquired_at_ms, expires_at_ms, last_heartbeat_ms, state
         FROM classifier_leases WHERE id = ?1",
        params![id],
        map_row,
    )
    .optional()
    .map_err(Into::into)
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        holder: row.get(1)?,
        acquired_at_ms: row.get(2)?,
        expires_at_ms: row.get(3)?,
        last_heartbeat_ms: row.get(4)?,
        state: row.get(5)?,
    })
}

fn mark(conn: &Connection, id: &str, state: &str) -> Result<(), Error> {
    conn.execute(
        "UPDATE classifier_leases SET state = ?1 WHERE id = ?2",
        params![state, id],
    )?;
    Ok(())
}

fn lease_from(row: Row) -> Result<ClassifierLease, Error> {
    let holder = SessionRef::new(row.holder)
        .map_err(|err| Error::Health(crewmux_health::Error::Rejected(err.to_string())))?;
    Ok(ClassifierLease {
        id: row.id,
        holder,
        acquired_at_ms: row.acquired_at_ms,
        expires_at_ms: row.expires_at_ms,
        last_heartbeat_ms: row.last_heartbeat_ms,
        state: parse_state(&row.state)?,
    })
}

fn parse_state(state: &str) -> Result<LeaseState, Error> {
    match state {
        "active" => Ok(LeaseState::Active),
        "expired" => Ok(LeaseState::Expired),
        "reclaimed" => Ok(LeaseState::Released),
        other => Err(Error::Health(crewmux_health::Error::Rejected(format!(
            "unknown lease state {other}"
        )))),
    }
}

fn constraint(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::ConstraintViolation
    )
}
