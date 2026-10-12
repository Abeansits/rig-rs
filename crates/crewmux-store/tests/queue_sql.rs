//! Crash-safe queue snapshots and the terminal retention move.
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers abort when a fixture cannot be built"
)]

use std::path::PathBuf;

use crewmux_core::Timestamp;
use crewmux_messaging::{Actor, NewItem, QueueBook, QueueState, SessionRef, Update};
use crewmux_store::{Store, archive_terminal, load_book, replace_book};
use crewmux_types::QItemId;

fn at(millis: i64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn session(text: &str) -> SessionRef {
    SessionRef::new(text).expect("session")
}

fn insert(book: &mut QueueBook, id: &str, when: Timestamp) -> QItemId {
    let qid = QItemId::new(id).expect("id");
    book.create(
        NewItem {
            id: Some(qid.clone()),
            source: session("alice@rig"),
            destination: session("bob@rig"),
            body: id.to_owned(),
            priority: 0,
            tier: Some("routine".to_owned()),
            tags: Vec::new(),
            handed_off_from: None,
            last_nudge: None,
            explicit_human: false,
            fingerprint: String::new(),
        },
        when,
    )
    .expect("create");
    qid
}

fn close_done(book: &mut QueueBook, id: &QItemId, when: Timestamp) {
    book.claim(id, &session("bob@rig"), when, None)
        .expect("claim");
    book.update(
        id,
        Update {
            state: Some(QueueState::Done),
            actor: Actor::Session(session("bob@rig")),
            note: "done".to_owned(),
            closure_reason: Some("no-follow-on".to_owned()),
            closure_target: None,
            reopen: false,
            blocked_on: None,
        },
        when,
    )
    .expect("done");
}

#[test]
fn kill_daemon_mid_handoff_keeps_the_before_state() {
    let store = Store::open(PathBuf::from(":memory:").as_path()).expect("open");
    store
        .write(|conn| {
            let mut book = QueueBook::new();
            let id = insert(&mut book, "baton-1", at(1_000));
            let crashed = replace_book(conn, &book, Some("after-items")).expect_err("items");
            assert_eq!(crashed.to_string(), "crash hook after-items");
            assert_eq!(load_book(conn).expect("empty").items().len(), 0);
            replace_book(conn, &book, None).expect("commit");
            assert_eq!(
                load_book(conn).expect("live").get(&id).expect("row").state,
                QueueState::Pending
            );
            let mut next = load_book(conn).expect("again");
            next.claim(&id, &session("bob@rig"), at(2_000), None)
                .expect("claim");
            let crashed = replace_book(conn, &next, Some("before-commit")).expect_err("commit");
            assert_eq!(crashed.to_string(), "crash hook before-commit");
            let still = load_book(conn).expect("rolled");
            assert_eq!(still.get(&id).expect("row").state, QueueState::Pending);
            assert!(still.get(&id).expect("row").claimed_by.is_none());
            Ok(())
        })
        .expect("write");
}

#[test]
fn archive_moves_only_old_terminal_rows() {
    let store = Store::open(PathBuf::from(":memory:").as_path()).expect("open");
    store
        .write(|conn| {
            let mut book = QueueBook::new();
            let done_id = insert(&mut book, "done-old", at(1_000));
            close_done(&mut book, &done_id, at(1_000));
            let live_id = insert(&mut book, "still-pending", at(5_000));
            let handed_id = insert(&mut book, "handed-old", at(1_000));
            book.handoff(
                &handed_id,
                &session("carol@rig"),
                &session("bob@rig"),
                at(1_000),
                false,
            )
            .expect("handoff");
            replace_book(conn, &book, None).expect("save");
            let moved = archive_terminal(conn, at(2_000)).expect("archive");
            assert!(moved.iter().any(|id| id == "done-old"));
            assert!(moved.iter().any(|id| id == "handed-old"));
            assert!(!moved.iter().any(|id| id == "still-pending"));
            let left = load_book(conn).expect("left");
            assert!(left.get(&done_id).is_none());
            assert!(left.get(&handed_id).is_none());
            assert_eq!(
                left.get(&live_id).expect("pending").state,
                QueueState::Pending
            );
            let successor = left.items().into_iter().any(|item| {
                item.handed_off_from
                    .as_ref()
                    .is_some_and(|from| from.as_str() == "handed-old")
            });
            assert!(successor, "the pending successor stays live");
            let archived: i64 =
                conn.query_row("SELECT COUNT(*) FROM queue_items_archive", [], |row| {
                    row.get(0)
                })?;
            assert!(archived >= 2, "{archived}");
            Ok(())
        })
        .expect("write");
}
