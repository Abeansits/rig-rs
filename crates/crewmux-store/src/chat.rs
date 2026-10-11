// Derived from openrig packages/daemon/src/domain/chat-repository.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Chat rows. Without a cursor the page is the newest `limit`, oldest first.
//! With a cursor the page continues forward from that id.

use std::str::FromStr;

use crewmux_core::Timestamp;
use crewmux_core::chat::{
    ChatError, ChatMessage, ChatRepo, HistoryPage, HistoryQuery, NewChat, NewTopic,
};
use crewmux_types::{ChatCursor, ChatId, RigId, SenderRef, TopicId};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, params_from_iter};

use crate::error::Error;
use crate::repo::Db;
use crate::store::Store;

const MAX_LIMIT: u32 = 10_000;

impl ChatRepo for Db<'_> {
    fn post(&self, message: NewChat) -> Result<ChatMessage, ChatError> {
        insert(
            self,
            &message.rig,
            &message.sender,
            &message.body,
            message.at,
            None,
        )
    }

    fn history(&self, query: &HistoryQuery) -> Result<HistoryPage, ChatError> {
        read_page(self, query)
    }

    fn set_topic(&self, marker: NewTopic) -> Result<ChatMessage, ChatError> {
        insert(
            self,
            &marker.rig,
            &marker.sender,
            &marker.body,
            marker.at,
            Some(&marker.name),
        )
    }

    fn has_topic(&self, rig: &RigId, name: &TopicId) -> Result<bool, ChatError> {
        let found: Option<i64> = self
            .query_row(
                "SELECT 1 FROM chat_messages WHERE rig_id = ?1 AND topic = ?2 LIMIT 1",
                rusqlite::params![rig.to_string(), name.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(chat_db)?;
        Ok(found.is_some())
    }

    fn clear(&self, rig: &RigId) -> Result<u64, ChatError> {
        let deleted = self
            .execute(
                "DELETE FROM chat_messages WHERE rig_id = ?1",
                rusqlite::params![rig.to_string()],
            )
            .map_err(chat_db)?;
        u64::try_from(deleted).map_err(|err| ChatError::Store(err.to_string()))
    }
}

impl ChatRepo for Store {
    fn post(&self, message: NewChat) -> Result<ChatMessage, ChatError> {
        self.write(move |conn| Db::new(conn).post(message).map_err(Error::from))
            .map_err(Error::into_chat)
    }

    fn history(&self, query: &HistoryQuery) -> Result<HistoryPage, ChatError> {
        let query = query.clone();
        self.read(move |conn| Db::new(conn).history(&query).map_err(Error::from))
            .map_err(Error::into_chat)
    }

    fn set_topic(&self, marker: NewTopic) -> Result<ChatMessage, ChatError> {
        self.write(move |conn| Db::new(conn).set_topic(marker).map_err(Error::from))
            .map_err(Error::into_chat)
    }

    fn has_topic(&self, rig: &RigId, name: &TopicId) -> Result<bool, ChatError> {
        let rig = *rig;
        let name = name.clone();
        self.read(move |conn| Db::new(conn).has_topic(&rig, &name).map_err(Error::from))
            .map_err(Error::into_chat)
    }

    fn clear(&self, rig: &RigId) -> Result<u64, ChatError> {
        let rig = *rig;
        self.write(move |conn| Db::new(conn).clear(&rig).map_err(Error::from))
            .map_err(Error::into_chat)
    }
}

fn insert(
    conn: &Connection,
    rig: &RigId,
    sender: &SenderRef,
    body: &str,
    at: Timestamp,
    topic: Option<&TopicId>,
) -> Result<ChatMessage, ChatError> {
    let id = ChatId::generate();
    let topic_text = topic.map(TopicId::as_str);
    conn.execute(
        "INSERT INTO chat_messages (id, rig_id, sender, body, created_ms, topic)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id.to_string(),
            rig.to_string(),
            sender.as_str(),
            body,
            at.millis(),
            topic_text
        ],
    )
    .map_err(chat_db)?;
    Ok(ChatMessage {
        id,
        rig: *rig,
        sender: sender.clone(),
        body: body.to_owned(),
        at,
        topic: topic.cloned(),
    })
}

fn read_page(conn: &Connection, query: &HistoryQuery) -> Result<HistoryPage, ChatError> {
    let window = match &query.topic {
        Some(topic) => Some(topic_window(conn, &query.rig, topic)?),
        None => None,
    };
    if query.limit == 0 {
        return Ok(HistoryPage {
            messages: Vec::new(),
            next: None,
        });
    }
    let limit = query.limit.min(MAX_LIMIT);
    let forward = query.after.is_some();
    let fetch = if forward {
        limit.saturating_add(1)
    } else {
        limit
    };
    let (sql, params) = statement(query, window.as_ref(), forward, fetch);
    let mut stmt = conn.prepare(&sql).map_err(chat_db)?;
    let mut rows = stmt
        .query_map(params_from_iter(params.iter()), map_row)
        .map_err(chat_db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(chat_db)?;
    if !forward {
        rows.reverse();
    }
    let next = trim_forward(&mut rows, limit, forward);
    Ok(HistoryPage {
        messages: rows,
        next,
    })
}

struct Window {
    start: String,
    end: Option<String>,
}

fn topic_window(conn: &Connection, rig: &RigId, topic: &TopicId) -> Result<Window, ChatError> {
    let start: Option<String> = conn
        .query_row(
            "SELECT id FROM chat_messages
             WHERE rig_id = ?1 AND topic = ?2
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![rig.to_string(), topic.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(chat_db)?;
    let Some(start) = start else {
        return Err(ChatError::UnknownTopic(topic.as_str().to_owned()));
    };
    let end: Option<String> = conn
        .query_row(
            "SELECT id FROM chat_messages
             WHERE rig_id = ?1 AND topic IS NOT NULL AND id > ?2
             ORDER BY id ASC LIMIT 1",
            rusqlite::params![rig.to_string(), start],
            |row| row.get(0),
        )
        .optional()
        .map_err(chat_db)?;
    Ok(Window { start, end })
}

fn statement(
    query: &HistoryQuery,
    window: Option<&Window>,
    forward: bool,
    fetch: u32,
) -> (String, Vec<SqlVal>) {
    let mut sql = String::from(
        "SELECT id, rig_id, sender, body, created_ms, topic FROM chat_messages WHERE rig_id = ?",
    );
    let mut params = vec![SqlVal::Text(query.rig.to_string())];
    if let Some(window) = window {
        sql.push_str(" AND id >= ?");
        params.push(SqlVal::Text(window.start.clone()));
        if let Some(end) = &window.end {
            sql.push_str(" AND id < ?");
            params.push(SqlVal::Text(end.clone()));
        }
    }
    if let Some(after) = &query.after {
        sql.push_str(" AND id > ?");
        params.push(SqlVal::Text(after.as_str().to_owned()));
    }
    if let Some(since) = query.since {
        sql.push_str(" AND created_ms >= ?");
        params.push(SqlVal::Int(since.millis()));
    }
    if let Some(sender) = &query.sender {
        sql.push_str(" AND sender = ?");
        params.push(SqlVal::Text(sender.as_str().to_owned()));
    }
    if forward {
        sql.push_str(" ORDER BY id ASC LIMIT ?");
    } else {
        sql.push_str(" ORDER BY id DESC LIMIT ?");
    }
    params.push(SqlVal::Int(i64::from(fetch)));
    (sql, params)
}

fn trim_forward(rows: &mut Vec<ChatMessage>, limit: u32, forward: bool) -> Option<ChatCursor> {
    if !forward {
        return None;
    }
    let limit = usize::try_from(limit).unwrap_or(0);
    if rows.len() <= limit {
        return None;
    }
    rows.truncate(limit);
    rows.last().map(|row| ChatCursor::from_chat_id(row.id))
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<ChatMessage> {
    let id: String = row.get(0)?;
    let rig: String = row.get(1)?;
    let sender: String = row.get(2)?;
    let body: String = row.get(3)?;
    let created_ms: i64 = row.get(4)?;
    let topic: Option<String> = row.get(5)?;
    Ok(ChatMessage {
        id: parse_id(&id)?,
        rig: parse_rig(&rig)?,
        sender: parse_sender(&sender)?,
        body,
        at: Timestamp::from_millis(created_ms),
        topic: topic.map(|text| parse_topic(&text)).transpose()?,
    })
}

fn parse_id(text: &str) -> rusqlite::Result<ChatId> {
    ChatId::from_str(text).map_err(|err| sql_err(err.to_string()))
}

fn parse_rig(text: &str) -> rusqlite::Result<RigId> {
    RigId::from_str(text).map_err(|err| sql_err(err.to_string()))
}

fn parse_sender(text: &str) -> rusqlite::Result<SenderRef> {
    SenderRef::parse(text).map_err(|err| sql_err(err.to_string()))
}

fn parse_topic(text: &str) -> rusqlite::Result<TopicId> {
    TopicId::new(text).map_err(|err| sql_err(err.to_string()))
}

fn sql_err(message: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(message)))
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err passes rusqlite::Error by value and Display does not consume it"
)]
fn chat_db(err: rusqlite::Error) -> ChatError {
    let text = err.to_string();
    if text.contains("FOREIGN KEY") {
        ChatError::UnknownRig
    } else {
        ChatError::Store(text)
    }
}

enum SqlVal {
    Text(String),
    Int(i64),
}

impl ToSql for SqlVal {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            Self::Text(text) => text.to_sql(),
            Self::Int(value) => value.to_sql(),
        }
    }
}
