//! Classifier lease and stream rows.

use crewmux_health::{ClassifierLease, DEFAULT_LEASE_TTL_MS, LeaseState};
use crewmux_store::{
    LeaseClock, NewStream, Store, lease_acquire, lease_active, lease_evaluate, lease_heartbeat,
    lease_reclaim, stream_archive, stream_emit, stream_list,
};

fn clock(now_ms: i64) -> LeaseClock {
    LeaseClock {
        now_ms,
        ttl_ms: DEFAULT_LEASE_TTL_MS,
    }
}

macro_rules! open {
    () => {
        Store::open(std::path::Path::new(":memory:")).expect("db")
    };
}

#[test]
fn acquire_is_idempotent_for_the_same_session() {
    let store = open!();
    let first = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let second = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(2_000)))
        .expect("again");
    assert_eq!(first.id, second.id);
    assert_eq!(second.expires_at_ms, first.expires_at_ms);
    assert_eq!(second.state, LeaseState::Active);
}

#[test]
fn second_session_is_lease_held() {
    let store = open!();
    store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let err = store
        .write(|conn| lease_acquire(conn, "bob@rig", clock(1_000)))
        .expect_err("held");
    assert!(err.to_string().contains("lease_held"));
}

#[test]
fn heartbeat_extends_the_ttl() {
    let store = open!();
    let lease = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let id = lease.id.clone();
    let beat = store
        .write(move |conn| lease_heartbeat(conn, &id, "alice@rig", clock(5_000)))
        .expect("beat");
    assert!(beat.expires_at_ms > lease.expires_at_ms);
    assert_eq!(beat.last_heartbeat_ms, 5_000);
}

#[test]
fn heartbeat_mismatch_is_forbidden() {
    let store = open!();
    let lease = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let id = lease.id;
    let err = store
        .write(move |conn| lease_heartbeat(conn, &id, "bob@rig", clock(2_000)))
        .expect_err("mismatch");
    assert!(err.to_string().contains("lease_session_mismatch"));
}

#[test]
fn evaluate_marks_ttl_expired_without_reclaim() {
    let store = open!();
    let lease = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let alive = store
        .write(|conn| lease_evaluate(conn, 1_000))
        .expect("alive");
    assert_eq!(alive, 0);
    let expires = lease.expires_at_ms;
    let marked = store
        .write(move |conn| lease_evaluate(conn, expires.saturating_add(1)))
        .expect("expire");
    assert_eq!(marked, 1);
    let row = store.write(|conn| lease_active(conn)).expect("read");
    assert!(row.is_none());
    let id = lease.id;
    let state = store
        .read(move |conn| {
            let state: String = conn.query_row(
                "SELECT state FROM classifier_leases WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )?;
            Ok(state)
        })
        .expect("state");
    assert_eq!(state, "expired");
}

#[test]
fn new_session_acquires_after_expiry() {
    let store = open!();
    let lease = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let expires = lease.expires_at_ms;
    store
        .write(move |conn| lease_evaluate(conn, expires.saturating_add(1)))
        .expect("expire");
    let next = store
        .write(move |conn| lease_acquire(conn, "bob@rig", clock(expires.saturating_add(2))))
        .expect("next");
    assert_ne!(next.id, lease.id);
    assert_eq!(next.holder.as_str(), "bob@rig");
}

#[test]
fn reclaim_if_dead_refuses_a_live_holder() {
    let store = open!();
    store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let err = store
        .write(|conn| lease_reclaim(conn, true, 1_000))
        .expect_err("alive");
    assert!(err.to_string().contains("lease_still_active"));
    let taken: ClassifierLease = store
        .write(|conn| lease_reclaim(conn, false, 1_000))
        .expect("reclaim");
    assert_eq!(taken.state, LeaseState::Released);
    let json = serde_json::to_value(&taken).expect("json");
    assert_eq!(json["state"], "reclaimed");
}

#[test]
fn partial_unique_index_rejects_a_second_active_row() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rig.db");
    let store = Store::open(&path).expect("db");
    store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let conn = rusqlite::Connection::open(&path).expect("second");
    let err = conn
        .execute(
            "INSERT INTO classifier_leases (id, holder, acquired_at_ms, expires_at_ms, last_heartbeat_ms, state)
             VALUES ('dup', 'bob@rig', 1, 2, 1, 'active')",
            [],
        )
        .expect_err("unique");
    assert!(err.to_string().contains("UNIQUE"));
}

#[test]
fn stream_emit_is_idempotent_and_list_hides_archived() {
    let store = open!();
    let first = store
        .write(|conn| {
            stream_emit(
                conn,
                &NewStream {
                    id: Some("item-1".into()),
                    kind: "note".into(),
                    body: "one".into(),
                    source: "alice@rig".into(),
                    at_ms: 10,
                },
            )
        })
        .expect("emit");
    assert!(first.created);
    let second = store
        .write(|conn| {
            stream_emit(
                conn,
                &NewStream {
                    id: Some("item-1".into()),
                    kind: "note".into(),
                    body: "changed".into(),
                    source: "alice@rig".into(),
                    at_ms: 11,
                },
            )
        })
        .expect("again");
    assert!(!second.created);
    assert_eq!(second.item.body, "one");
    store
        .write(|conn| {
            stream_emit(
                conn,
                &NewStream {
                    id: Some("item-2".into()),
                    kind: "note".into(),
                    body: "two".into(),
                    source: "alice@rig".into(),
                    at_ms: 12,
                },
            )
        })
        .expect("second item");
    let before = store
        .write(|conn| stream_list(conn, 50, false))
        .expect("before archive");
    assert_eq!(
        before
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["item-1", "item-2"]
    );
    store
        .write(|conn| stream_archive(conn, "item-1"))
        .expect("archive");
    let listed = store
        .write(|conn| stream_list(conn, 50, false))
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "item-2");
    assert!(listed[0].seq > first.item.seq);
}

#[test]
fn heartbeat_past_ttl_does_not_revive_the_lease() {
    let store = open!();
    let lease = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let expires = lease.expires_at_ms;
    let id = lease.id;
    let lookup = id.clone();
    let err = store
        .write(move |conn| {
            lease_heartbeat(conn, &id, "alice@rig", clock(expires.saturating_add(1)))
        })
        .expect_err("expired");
    assert!(err.to_string().contains("lease_expired"));
    let (state, stored_expires) = store
        .read(move |conn| {
            let pair: (String, i64) = conn.query_row(
                "SELECT state, expires_at_ms FROM classifier_leases WHERE id = ?1",
                rusqlite::params![lookup],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok(pair)
        })
        .expect("state");
    assert_eq!(state, "expired");
    assert_eq!(stored_expires, expires);
}

#[test]
fn same_session_acquire_after_ttl_issues_a_new_lease() {
    let store = open!();
    let first = store
        .write(|conn| lease_acquire(conn, "alice@rig", clock(1_000)))
        .expect("acquire");
    let expires = first.expires_at_ms;
    let previous = first.id;
    let second = store
        .write(move |conn| lease_acquire(conn, "alice@rig", clock(expires.saturating_add(1))))
        .expect("next");
    assert_ne!(second.id, previous);
    assert_eq!(second.holder.as_str(), "alice@rig");
    let state = store
        .read(move |conn| {
            let state: String = conn.query_row(
                "SELECT state FROM classifier_leases WHERE id = ?1",
                rusqlite::params![previous],
                |row| row.get(0),
            )?;
            Ok(state)
        })
        .expect("state");
    assert_eq!(state, "expired");
}
