//! Seat tokens. The daemon writes one after a seat launch and reads it to
//! authenticate chat.

use crewmux_core::Error as CoreError;
use crewmux_types::SeatId;
use rusqlite::{Connection, OptionalExtension, params};

/// Store `token` for `seat`. Replaces a token already stored for that seat.
///
/// # Errors
///
/// Returns [`CoreError::Store`] when the row cannot be written. The seat must
/// already exist.
pub fn put_seat_token(conn: &Connection, seat: &SeatId, token: &str) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO seat_tokens (seat_id, token) VALUES (?1, ?2)
         ON CONFLICT(seat_id) DO UPDATE SET token = excluded.token",
        params![seat.to_string(), token],
    )
    .map_err(db)?;
    Ok(())
}

/// The token stored for `seat`, when a launch recorded one.
///
/// # Errors
///
/// Returns [`CoreError::Store`] when the read fails.
pub fn seat_token(conn: &Connection, seat: &SeatId) -> Result<Option<String>, CoreError> {
    conn.query_row(
        "SELECT token FROM seat_tokens WHERE seat_id = ?1",
        params![seat.to_string()],
        |row| row.get(0),
    )
    .optional()
    .map_err(db)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err passes rusqlite::Error by value and Display does not consume it"
)]
fn db(err: rusqlite::Error) -> CoreError {
    CoreError::Store(err.to_string())
}
