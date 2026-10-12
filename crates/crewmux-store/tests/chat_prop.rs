//! History filters against a naive in-memory reference. The store assigns ids;
//! the reference only applies the window, cursor, instant, and sender rules.

use std::path::Path;

use crewmux_core::chat::{ChatError, ChatRepo, HistoryQuery, NewChat, NewTopic};
use crewmux_core::{Rig, RigRepo, Timestamp};
use crewmux_store::{Db, Store};
use crewmux_types::{ChatCursor, RigId, RigName, SenderRef, TopicId};
use proptest::prelude::*;

const MAX_LIMIT: u32 = 10_000;
const BASE_MS: i64 = 1_700_000_000_000;

#[derive(Clone, Debug)]
struct Row {
    id: String,
    sender: String,
    at: i64,
    topic: Option<String>,
}

#[derive(Clone, Debug)]
struct Ask {
    topic: Option<String>,
    after: Option<usize>,
    since: Option<i64>,
    sender: Option<String>,
    limit: u32,
}

fn reference(rows: &[Row], ask: &Ask) -> Result<(Vec<String>, Option<String>), ChatError> {
    let mut view: Vec<&Row> = rows.iter().collect();
    if let Some(topic) = &ask.topic {
        let start = view
            .iter()
            .rev()
            .find(|row| row.topic.as_deref() == Some(topic.as_str()));
        let Some(start) = start else {
            return Err(ChatError::UnknownTopic(topic.clone()));
        };
        let start_id = start.id.clone();
        let end = view
            .iter()
            .find(|row| row.topic.is_some() && row.id.as_str() > start_id.as_str())
            .map(|row| row.id.clone());
        view.retain(|row| {
            row.id.as_str() >= start_id.as_str()
                && end
                    .as_ref()
                    .is_none_or(|end| row.id.as_str() < end.as_str())
        });
    }
    if let Some(after) = ask.after.and_then(|index| rows.get(index)) {
        view.retain(|row| row.id.as_str() > after.id.as_str());
    }
    if let Some(since) = ask.since {
        view.retain(|row| row.at >= since);
    }
    if let Some(sender) = &ask.sender {
        view.retain(|row| row.sender == *sender);
    }
    let limit = usize::try_from(ask.limit.min(MAX_LIMIT)).unwrap_or(0);
    if ask.after.is_none() {
        view.sort_by(|left, right| right.id.cmp(&left.id));
        view.truncate(limit);
        view.reverse();
        return Ok((view.iter().map(|row| row.id.clone()).collect(), None));
    }
    view.sort_by(|left, right| left.id.cmp(&right.id));
    if view.len() <= limit {
        return Ok((view.iter().map(|row| row.id.clone()).collect(), None));
    }
    view.truncate(limit);
    let next = view.last().map(|row| row.id.clone());
    Ok((view.iter().map(|row| row.id.clone()).collect(), next))
}

fn who(index: u8) -> String {
    match index {
        0 => "alice",
        1 => "bob",
        _ => "host",
    }
    .to_owned()
}

fn topic_name(index: u8) -> String {
    match index {
        0 => "review",
        1 => "deploy",
        _ => "other",
    }
    .to_owned()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]
    #[test]
    fn filters_match_the_reference(
        steps in prop::collection::vec((0..3u8, "[a-z]{1,8}", 0..6i64, prop::option::of(0..3u8)), 0..8),
        topic in prop::option::of(prop::sample::select(vec!["review".to_owned(), "deploy".to_owned(), "missing".to_owned()])),
        after in prop::option::of(0..8usize),
        since in prop::option::of(0..6i64),
        sender in prop::option::of(0..3u8),
        limit in 0..5u32,
    ) {
        let store = Store::open(Path::new(":memory:")).expect("open");
        let rig = RigId::new();
        let saved = rig;
        store.write(move |conn| {
            Db::new(conn).upsert(&Rig {
                id: saved,
                name: RigName::new("prop").expect("name"),
                created_at: Timestamp::from_millis(0),
            })?;
            Ok(())
        }).expect("rig");
        let mut rows = Vec::new();
        for (who_i, body, offset, marker) in steps {
            let at = Timestamp::from_millis(BASE_MS.saturating_add(offset.saturating_mul(1000)));
            let person = who(who_i);
            let stored = if let Some(marker) = marker {
                store.set_topic(NewTopic {
                    rig,
                    sender: SenderRef::parse(&person).expect("sender"),
                    name: TopicId::new(topic_name(marker)).expect("topic"),
                    body: body.clone(),
                    at,
                }).expect("topic")
            } else {
                store.post(NewChat {
                    rig,
                    sender: SenderRef::parse(&person).expect("sender"),
                    body: body.clone(),
                    at,
                }).expect("post")
            };
            rows.push(Row {
                id: stored.id.to_string(),
                sender: person,
                at: at.millis(),
                topic: stored.topic.map(|topic| topic.as_str().to_owned()),
            });
        }
        let ask = Ask {
            topic,
            after: after.filter(|index| rows.get(*index).is_some()),
            since: since.map(|offset| BASE_MS.saturating_add(offset.saturating_mul(1000))),
            sender: sender.map(who),
            limit,
        };
        let expected = reference(&rows, &ask);
        let query = HistoryQuery {
            rig,
            topic: ask.topic.as_deref().map(|text| TopicId::new(text).expect("topic")),
            after: ask.after.and_then(|index| rows.get(index)).map(|row| ChatCursor::new(&row.id).expect("cursor")),
            since: ask.since.map(Timestamp::from_millis),
            sender: ask.sender.as_deref().map(|text| SenderRef::parse(text).expect("sender")),
            limit: ask.limit,
        };
        let got = store.history(&query);
        match (expected, got) {
            (Ok((ids, next)), Ok(page)) => {
                let got_ids: Vec<String> = page.messages.iter().map(|row| row.id.to_string()).collect();
                prop_assert_eq!(got_ids, ids);
                prop_assert_eq!(page.next.as_ref().map(ChatCursor::as_str), next.as_deref());
            }
            (Err(left), Err(right)) => prop_assert_eq!(left, right),
            (left, right) => prop_assert!(false, "reference {left:?} store {right:?}"),
        }
    }
}
