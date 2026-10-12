//! Chat repository behaviour from `chat-repository.test.ts`, plus the newest-page
//! and unknown-topic rules, against one `:memory:` connection.
#![expect(
    clippy::expect_used,
    reason = "setup failures abort the integration test"
)]

use std::path::Path;

use crewmux_core::chat::{
    ChatError, ChatMessage, ChatRepo, HistoryPage, HistoryQuery, NewChat, NewTopic, parse_since,
};
use crewmux_core::{Rig, RigRepo, Timestamp};
use crewmux_store::{Db, Store};
use crewmux_types::{ChatCursor, ChatId, RigId, RigName, SenderRef, TopicId};

fn open() -> (Store, RigId) {
    let store = Store::open(Path::new(":memory:")).expect("open");
    let rig = RigId::new();
    let saved = rig;
    store
        .write(move |conn| {
            Db::new(conn).upsert(&Rig {
                id: saved,
                name: RigName::new("test-rig").expect("name"),
                created_at: Timestamp::from_millis(0),
            })?;
            Ok(())
        })
        .expect("rig");
    (store, rig)
}

fn sender(name: &str) -> SenderRef {
    SenderRef::parse(name).expect("sender")
}

fn say(store: &Store, rig: RigId, who: &str, body: &str) -> ChatMessage {
    store
        .post(NewChat {
            rig,
            sender: sender(who),
            body: body.to_owned(),
            at: Timestamp::from_millis(1_700_000_000_000),
        })
        .expect("post")
}

fn topic(store: &Store, rig: RigId, who: &str, name: &str, body: &str) -> ChatMessage {
    store
        .set_topic(NewTopic {
            rig,
            sender: sender(who),
            name: TopicId::new(name).expect("topic"),
            body: body.to_owned(),
            at: Timestamp::from_millis(1_700_000_000_000),
        })
        .expect("topic")
}

fn query(rig: RigId) -> HistoryQuery {
    HistoryQuery {
        rig,
        topic: None,
        after: None,
        since: None,
        sender: None,
        limit: 100,
    }
}

fn bodies(page: &HistoryPage) -> Vec<&str> {
    page.messages.iter().map(|row| row.body.as_str()).collect()
}

fn page(store: &Store, query: &HistoryQuery) -> HistoryPage {
    store.history(query).expect("history")
}

fn stamp(store: &Store, id: ChatId, text: &str) {
    let at = parse_since(text, Timestamp::from_millis(0))
        .expect("stamp")
        .millis();
    let key = id.to_string();
    store
        .write(move |conn| {
            conn.execute(
                "UPDATE chat_messages SET created_ms = ?1 WHERE id = ?2",
                rusqlite::params![at, key],
            )?;
            Ok(())
        })
        .expect("update");
}

#[test]
fn same_day_iso_since_uses_utc_chronology() {
    let (store, rig) = open();
    let early = say(&store, rig, "alice", "before cutoff");
    let boundary = say(&store, rig, "alice", "at cutoff");
    let late = say(&store, rig, "bob", "after cutoff");
    stamp(&store, early.id, "2026-09-30 10:59:59");
    stamp(&store, boundary.id, "2026-09-30 11:00:00");
    stamp(&store, late.id, "2026-09-30 11:00:01");
    for since in [
        "2026-09-30T11:00:00Z",
        "2026-09-30 11:00:00",
        "2026-09-30T07:00:00-04:00",
        "2026-09-30T13:00:00+02:00",
    ] {
        let mut q = query(rig);
        q.since = Some(parse_since(since, Timestamp::from_millis(0)).expect(since));
        assert_eq!(
            bodies(&page(&store, &q)),
            ["at cutoff", "after cutoff"],
            "{since}"
        );
    }
    let mut fraction = query(rig);
    fraction.since =
        Some(parse_since("2026-09-30T11:00:00.500Z", Timestamp::from_millis(0)).expect("fraction"));
    assert_eq!(bodies(&page(&store, &fraction)), ["after cutoff"]);
    let mut combined = query(rig);
    combined.since =
        Some(parse_since("2026-09-30T11:00:00Z", Timestamp::from_millis(0)).expect("utc"));
    combined.sender = Some(sender("alice"));
    combined.after = Some(ChatCursor::from_chat_id(early.id));
    combined.limit = 1;
    let got = page(&store, &combined);
    assert_eq!(bodies(&got), ["at cutoff"]);
    assert!(got.next.is_none());
}

#[test]
fn send_persists_with_ulid() {
    let (store, rig) = open();
    let msg = say(&store, rig, "alice", "hello world");
    assert_eq!(msg.id.to_string().len(), 26);
    assert_eq!(msg.rig, rig);
    assert_eq!(msg.sender.as_str(), "alice");
    assert_eq!(msg.body, "hello world");
    assert_eq!(msg.kind(), "message");
    assert!(msg.topic.is_none());
    assert!(msg.at.millis() > 0);
}

#[test]
fn history_returns_chronological_order() {
    let (store, rig) = open();
    say(&store, rig, "alice", "first");
    say(&store, rig, "bob", "second");
    say(&store, rig, "alice", "third");
    assert_eq!(
        bodies(&page(&store, &query(rig))),
        ["first", "second", "third"]
    );
}

#[test]
fn history_topic_returns_the_window() {
    let (store, rig) = open();
    say(&store, rig, "alice", "before topic");
    topic(&store, rig, "alice", "deploy", "starting deploy");
    say(&store, rig, "bob", "deploy message");
    say(&store, rig, "alice", "another deploy msg");
    topic(&store, rig, "bob", "standup", "daily standup");
    say(&store, rig, "bob", "standup message — should NOT appear");
    let mut q = query(rig);
    q.topic = Some(TopicId::new("deploy").expect("topic"));
    let got = page(&store, &q);
    let texts = bodies(&got);
    assert!(texts.contains(&"starting deploy"));
    assert!(texts.contains(&"deploy message"));
    assert!(texts.contains(&"another deploy msg"));
    assert!(!texts.contains(&"daily standup"));
    assert!(!texts.contains(&"standup message — should NOT appear"));
    assert!(!texts.contains(&"before topic"));
}

#[test]
fn send_topic_creates_topic_kind_message() {
    let (store, rig) = open();
    let msg = topic(&store, rig, "alice", "standup", "daily standup");
    assert_eq!(msg.kind(), "topic");
    assert_eq!(msg.topic.as_ref().map(TopicId::as_str), Some("standup"));
    assert_eq!(msg.body, "daily standup");
    assert_eq!(msg.sender.as_str(), "alice");
}

#[test]
fn clear_removes_all_messages() {
    let (store, rig) = open();
    say(&store, rig, "alice", "msg1");
    say(&store, rig, "bob", "msg2");
    say(&store, rig, "alice", "msg3");
    assert_eq!(store.clear(&rig).expect("clear"), 3);
    let cleared = page(&store, &query(rig));
    assert_eq!(cleared.messages.len(), 0);
}

#[test]
fn clear_returns_zero_for_empty_room() {
    let (store, rig) = open();
    assert_eq!(store.clear(&rig).expect("clear"), 0);
}

#[test]
fn clear_leaves_other_rigs() {
    let (store, rig) = open();
    let other = RigId::new();
    let saved = other;
    store
        .write(move |conn| {
            Db::new(conn).upsert(&Rig {
                id: saved,
                name: RigName::new("other-rig").expect("name"),
                created_at: Timestamp::from_millis(0),
            })?;
            Ok(())
        })
        .expect("other");
    say(&store, rig, "alice", "target rig msg");
    say(&store, other, "bob", "other rig msg");
    store.clear(&rig).expect("clear");
    let cleared = page(&store, &query(rig));
    assert_eq!(cleared.messages.len(), 0);
    assert_eq!(bodies(&page(&store, &query(other))), ["other rig msg"]);
}

#[test]
fn history_sender_returns_only_matching() {
    let (store, rig) = open();
    say(&store, rig, "alice", "alice msg 1");
    say(&store, rig, "bob", "bob msg 1");
    say(&store, rig, "alice", "alice msg 2");
    let mut q = query(rig);
    q.sender = Some(sender("alice"));
    let got = page(&store, &q);
    assert_eq!(got.messages.len(), 2);
    assert!(
        got.messages
            .iter()
            .all(|row| row.sender.as_str() == "alice")
    );
}

#[test]
fn history_since_returns_only_newer() {
    let (store, rig) = open();
    say(&store, rig, "alice", "msg after cutoff");
    say(&store, rig, "bob", "also after cutoff");
    let mut past = query(rig);
    past.since =
        Some(parse_since("2020-01-01T00:00:00Z", Timestamp::from_millis(0)).expect("past"));
    assert_eq!(page(&store, &past).messages.len(), 2);
    let mut future = query(rig);
    future.since =
        Some(parse_since("2099-01-01T00:00:00Z", Timestamp::from_millis(0)).expect("future"));
    let none = page(&store, &future);
    assert_eq!(none.messages.len(), 0);
}

#[test]
fn sender_and_after_compose() {
    let (store, rig) = open();
    let first = say(&store, rig, "alice", "alice before");
    say(&store, rig, "bob", "bob after");
    say(&store, rig, "alice", "alice after");
    let mut q = query(rig);
    q.after = Some(ChatCursor::from_chat_id(first.id));
    q.sender = Some(sender("alice"));
    assert_eq!(bodies(&page(&store, &q)), ["alice after"]);
}

#[test]
fn topic_and_after_compose() {
    let (store, rig) = open();
    topic(&store, rig, "host", "review", "");
    let first = say(&store, rig, "alice", "first review msg");
    say(&store, rig, "bob", "second review msg");
    let mut q = query(rig);
    q.topic = Some(TopicId::new("review").expect("topic"));
    q.after = Some(ChatCursor::from_chat_id(first.id));
    assert_eq!(bodies(&page(&store, &q)), ["second review msg"]);
}

#[test]
fn topic_and_sender_compose() {
    let (store, rig) = open();
    say(&store, rig, "alice", "before topic");
    topic(&store, rig, "host", "review", "");
    say(&store, rig, "alice", "alice review msg");
    say(&store, rig, "bob", "bob review msg");
    let mut q = query(rig);
    q.topic = Some(TopicId::new("review").expect("topic"));
    q.sender = Some(sender("alice"));
    assert_eq!(bodies(&page(&store, &q)), ["alice review msg"]);
}

#[test]
fn history_without_cursor_returns_the_newest_limit() {
    let (store, rig) = open();
    for body in ["a", "b", "c", "d", "e"] {
        say(&store, rig, "alice", body);
    }
    let mut q = query(rig);
    q.limit = 2;
    let got = page(&store, &q);
    assert_eq!(bodies(&got), ["d", "e"]);
    assert!(got.next.is_none());
    let oldest = page(&store, &query(rig)).messages;
    let cursor = ChatCursor::from_chat_id(oldest.get(2).expect("middle").id);
    let mut forward = query(rig);
    forward.after = Some(cursor);
    forward.limit = 1;
    let first = page(&store, &forward);
    assert_eq!(bodies(&first), ["d"]);
    let next = first.next.as_ref().map(ChatCursor::as_str);
    let id = first.messages.first().map(|row| row.id.to_string());
    assert_eq!(next, id.as_deref());
    let mut rest = query(rig);
    rest.after = first.next;
    let last = page(&store, &rest);
    assert_eq!(bodies(&last), ["e"]);
    assert!(last.next.is_none());
}

#[test]
fn unknown_topic_is_an_error() {
    let (store, rig) = open();
    say(&store, rig, "alice", "hello");
    let name = TopicId::new("missing").expect("topic");
    assert!(!store.has_topic(&rig, &name).expect("has"));
    let mut q = query(rig);
    q.topic = Some(name);
    assert_eq!(
        store.history(&q).expect_err("unknown"),
        ChatError::UnknownTopic("missing".to_owned())
    );
    q.limit = 0;
    assert_eq!(
        store.history(&q).expect_err("still unknown"),
        ChatError::UnknownTopic("missing".to_owned())
    );
}

#[test]
fn unparseable_since_is_not_an_empty_page() {
    let err = parse_since("garbage", Timestamp::from_millis(0)).expect_err("garbage");
    assert_eq!(err, ChatError::UnparseableSince("garbage".to_owned()));
}
