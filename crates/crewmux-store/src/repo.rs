//! Repository ports on the writer connection.

use crewmux_core::{
    DomainEvent, Error as CoreError, EventLog, Rig, RigRepo, Seat, SeatRepo, SeatState, Seq,
    SessionRepo, Timestamp,
};
use crewmux_types::{ReadinessCode, RigId, RigName, RuntimeKind, SeatId, SeatName, SessionId};
use rusqlite::{Connection, OptionalExtension, params};

/// Borrowed connection the core ports are implemented for.
pub struct Db<'a>(&'a Connection);

impl<'a> Db<'a> {
    /// Wrap a connection for the duration of one store job.
    #[must_use]
    pub const fn new(conn: &'a Connection) -> Self {
        Self(conn)
    }
}

impl std::ops::Deref for Db<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

/// Drop seats and their sessions so a stopped rig can launch again.
///
/// # Errors
///
/// Returns [`CoreError::Store`] when the delete fails.
pub fn clear_rig(conn: &Connection, rig: &RigId) -> Result<(), CoreError> {
    conn.execute(
        "DELETE FROM sessions WHERE seat_id IN (SELECT id FROM seats WHERE rig_id = ?1)",
        params![rig.to_string()],
    )
    .map_err(db)?;
    conn.execute(
        "DELETE FROM seats WHERE rig_id = ?1",
        params![rig.to_string()],
    )
    .map_err(db)?;
    Ok(())
}

impl RigRepo for Db<'_> {
    fn get(&self, id: &RigId) -> Result<Rig, CoreError> {
        self.query_row(
            "SELECT id, name, created_ms FROM rigs WHERE id = ?1",
            params![id.to_string()],
            map_rig,
        )
        .map_err(|err| missing(err, CoreError::RigNotFound(id.to_string())))
    }

    fn get_by_name(&self, name: &RigName) -> Result<Option<Rig>, CoreError> {
        self.query_row(
            "SELECT id, name, created_ms FROM rigs WHERE name = ?1",
            params![name.as_str()],
            map_rig,
        )
        .optional()
        .map_err(db)
    }

    fn upsert(&self, rig: &Rig) -> Result<(), CoreError> {
        self.execute(
            "INSERT INTO rigs (id, name, created_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, created_ms = excluded.created_ms",
            params![
                rig.id.to_string(),
                rig.name.as_str(),
                rig.created_at.millis()
            ],
        )
        .map_err(db)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<Rig>, CoreError> {
        let mut stmt = self
            .prepare("SELECT id, name, created_ms FROM rigs ORDER BY created_ms, name")
            .map_err(db)?;
        let rows = stmt
            .query_map([], map_rig)
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(rows)
    }
}

impl SeatRepo for Db<'_> {
    fn get(&self, id: &SeatId) -> Result<Seat, CoreError> {
        self.query_row(
            &seat_sql("WHERE id = ?1"),
            params![id.to_string()],
            map_seat,
        )
        .map_err(|err| missing(err, CoreError::SeatNotFound(id.to_string())))
    }

    fn list_by_rig(&self, rig: &RigId) -> Result<Vec<Seat>, CoreError> {
        let mut stmt = self
            .prepare(&seat_sql("WHERE rig_id = ?1 ORDER BY rowid"))
            .map_err(db)?;
        let rows = stmt
            .query_map(params![rig.to_string()], map_seat)
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(rows)
    }

    fn list_all(&self) -> Result<Vec<Seat>, CoreError> {
        let mut stmt = self.prepare(&seat_sql("ORDER BY rowid")).map_err(db)?;
        let rows = stmt
            .query_map([], map_seat)
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(rows)
    }

    fn upsert(&self, seat: &Seat) -> Result<(), CoreError> {
        let (state, code) = encode_state(&seat.state);
        self.execute(
            "INSERT INTO seats (
                id, rig_id, name, runtime, cwd, model, state, attention_code, reason, tmux_session, pane_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                rig_id = excluded.rig_id,
                name = excluded.name,
                runtime = excluded.runtime,
                cwd = excluded.cwd,
                model = excluded.model,
                state = excluded.state,
                attention_code = excluded.attention_code,
                reason = excluded.reason,
                tmux_session = excluded.tmux_session,
                pane_id = excluded.pane_id",
            params![
                seat.id.to_string(),
                seat.rig_id.to_string(),
                seat.name.as_str(),
                seat.runtime.as_str(),
                seat.cwd.as_ref().map(|path| path.display().to_string()),
                seat.model,
                state,
                code,
                seat.reason,
                seat.tmux_session,
                seat.pane_id,
            ],
        )
        .map_err(db)?;
        Ok(())
    }

    fn get_by_pane(&self, pane_id: &str) -> Result<Option<Seat>, CoreError> {
        self.query_row(&seat_sql("WHERE pane_id = ?1"), params![pane_id], map_seat)
            .optional()
            .map_err(db)
    }
}

impl SessionRepo for Db<'_> {
    fn insert(
        &self,
        id: &SessionId,
        seat: &SeatId,
        tmux_session: &str,
        pane_id: Option<&str>,
        at: Timestamp,
    ) -> Result<(), CoreError> {
        self.execute(
            "INSERT INTO sessions (id, seat_id, tmux_session, pane_id, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id.to_string(),
                seat.to_string(),
                tmux_session,
                pane_id,
                at.millis()
            ],
        )
        .map_err(db)?;
        Ok(())
    }
}

impl EventLog for Db<'_> {
    fn append(&self, event: &DomainEvent) -> Result<Seq, CoreError> {
        let payload =
            serde_json::to_string(event).map_err(|err| CoreError::Store(err.to_string()))?;
        self.execute(
            "INSERT INTO events (kind, payload, at_ms) VALUES (?1, ?2, ?3)",
            params![event.kind(), payload, event_at(event).millis()],
        )
        .map_err(db)?;
        let seq = u64::try_from(self.last_insert_rowid()).unwrap_or(0);
        Ok(Seq(seq))
    }

    fn list(&self) -> Result<Vec<DomainEvent>, CoreError> {
        let mut stmt = self
            .prepare("SELECT payload FROM events ORDER BY seq")
            .map_err(db)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let mut events = Vec::with_capacity(rows.len());
        for payload in rows {
            let event =
                serde_json::from_str(&payload).map_err(|err| CoreError::Store(err.to_string()))?;
            events.push(event);
        }
        Ok(events)
    }
}

fn seat_sql(tail: &str) -> String {
    format!(
        "SELECT id, rig_id, name, runtime, cwd, model, state, attention_code, reason, tmux_session, pane_id
         FROM seats {tail}"
    )
}

fn map_rig(row: &rusqlite::Row<'_>) -> rusqlite::Result<Rig> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let created: i64 = row.get(2)?;
    let id = parse_id(&id)?;
    let name = RigName::new(name).map_err(|err| sql_err(err.to_string()))?;
    Ok(Rig {
        id,
        name,
        created_at: Timestamp::from_millis(created),
    })
}

fn map_seat(row: &rusqlite::Row<'_>) -> rusqlite::Result<Seat> {
    let id: String = row.get(0)?;
    let rig: String = row.get(1)?;
    let name: String = row.get(2)?;
    let runtime: String = row.get(3)?;
    let cwd: Option<String> = row.get(4)?;
    let model: Option<String> = row.get(5)?;
    let state: String = row.get(6)?;
    let code: Option<String> = row.get(7)?;
    let reason: Option<String> = row.get(8)?;
    let tmux_session: Option<String> = row.get(9)?;
    let pane_id: Option<String> = row.get(10)?;
    Ok(Seat {
        id: parse_seat(&id)?,
        rig_id: parse_id(&rig)?,
        name: SeatName::new(name).map_err(|err| sql_err(err.to_string()))?,
        runtime: parse_runtime(&runtime)?,
        cwd: cwd.map(std::path::PathBuf::from),
        model,
        state: decode_state(&state, code.as_deref()).map_err(sql_err)?,
        reason,
        tmux_session,
        pane_id,
    })
}

fn parse_id(text: &str) -> rusqlite::Result<RigId> {
    text.parse::<RigId>()
        .map_err(|err| sql_err(err.to_string()))
}

fn parse_seat(text: &str) -> rusqlite::Result<SeatId> {
    text.parse::<SeatId>()
        .map_err(|err| sql_err(err.to_string()))
}

fn parse_runtime(text: &str) -> rusqlite::Result<RuntimeKind> {
    match text {
        "claude-code" => Ok(RuntimeKind::ClaudeCode),
        "codex" => Ok(RuntimeKind::Codex),
        "opencode" => Ok(RuntimeKind::OpenCode),
        "terminal" => Ok(RuntimeKind::Terminal),
        "stub" => Ok(RuntimeKind::Stub),
        other => Err(sql_err(format!("unknown runtime {other}"))),
    }
}

fn encode_state(state: &SeatState) -> (&'static str, Option<String>) {
    match state {
        SeatState::Attention(code) => ("attention", Some(code.as_str().to_owned())),
        SeatState::Planned
        | SeatState::Launching
        | SeatState::Ready
        | SeatState::Stopped
        | SeatState::Dead => (state.as_str(), None),
    }
}

fn decode_state(state: &str, code: Option<&str>) -> Result<SeatState, String> {
    match state {
        "planned" => Ok(SeatState::Planned),
        "launching" => Ok(SeatState::Launching),
        "ready" => Ok(SeatState::Ready),
        "stopped" => Ok(SeatState::Stopped),
        "dead" => Ok(SeatState::Dead),
        "attention" => Ok(SeatState::Attention(ReadinessCode::parse(
            code.unwrap_or("unrecognised_screen"),
        ))),
        other => Err(format!("unknown seat state {other}")),
    }
}

fn event_at(event: &DomainEvent) -> Timestamp {
    event.at()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err passes rusqlite::Error by value and Display does not consume it"
)]
fn db(err: rusqlite::Error) -> CoreError {
    CoreError::Store(err.to_string())
}

fn missing(err: rusqlite::Error, not_found: CoreError) -> CoreError {
    if matches!(err, rusqlite::Error::QueryReturnedNoRows) {
        not_found
    } else {
        db(err)
    }
}

fn sql_err(message: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(message)))
}
