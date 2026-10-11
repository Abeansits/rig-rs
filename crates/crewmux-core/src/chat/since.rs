// Derived from openrig packages/daemon/src/domain/chat-repository.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
// Derived from openrig packages/daemon/src/routes/chat.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Timestamp text compared as instants.
//!
//! Accepted shapes match the SQLite date subset the chat history route relies on:
//! a calendar date, a `YYYY-MM-DD HH:MM:SS` stamp, RFC 3339 with `Z` or a numeric
//! offset, and `now`. Naive stamps are UTC. A `+02:00` value equals its UTC twin.

use std::fmt::Write;

use crate::clock::Timestamp;

use super::error::ChatError;

/// Parse `value` the way chat history compares `since`.
///
/// # Errors
///
/// Returns [`ChatError::UnparseableSince`] when `value` is not one of the accepted shapes.
pub fn parse_since(value: &str, now: Timestamp) -> Result<Timestamp, ChatError> {
    if value == "now" {
        return Ok(now);
    }
    parse_instant(value).ok_or_else(|| ChatError::UnparseableSince(value.to_owned()))
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, truncated to a second.
#[must_use]
pub fn format_sqlite_utc(at: Timestamp) -> String {
    format_parts(at).unwrap_or_else(|| "1970-01-01 00:00:00".to_owned())
}

/// The history route's 400 text for a `since` value [`parse_since`] rejects.
#[must_use]
pub fn rejected_since(value: &str) -> String {
    format!(
        "since must be a datetime SQLite can parse, such as 'YYYY-MM-DD HH:MM:SS'; got '{}'",
        escape_controls(value)
    )
}

/// Render control characters as `\xNN` so a rejected `since` cannot smuggle a newline.
#[must_use]
pub fn escape_controls(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        let code = u32::from(ch);
        if code <= 0x1f || (0x7f..=0x9f).contains(&code) {
            let _ = write!(out, "\\x{code:02x}");
        } else {
            out.push(ch);
        }
    }
    out
}

fn parse_instant(value: &str) -> Option<Timestamp> {
    if value.is_empty() || value != value.trim() {
        return None;
    }
    let (body, offset_min) = split_zone(value)?;
    let parts = parse_body(body)?;
    let local = civil_to_unix_ms(parts)?;
    let offset_ms = i64::from(offset_min).checked_mul(60_000)?;
    local.checked_sub(offset_ms).map(Timestamp::from_millis)
}

fn split_zone(value: &str) -> Option<(&str, i32)> {
    if value.ends_with('Z') || value.ends_with('z') {
        let body = value.get(..value.len().saturating_sub(1))?;
        return Some((body, 0));
    }
    let mut sign_at = None;
    for (index, byte) in value.bytes().enumerate().skip(10) {
        if byte == b'+' || byte == b'-' {
            sign_at = Some(index);
        }
    }
    let Some(index) = sign_at else {
        return Some((value, 0));
    };
    let body = value.get(..index)?;
    let zone = value.get(index..)?;
    Some((body, parse_offset(zone)?))
}

fn parse_offset(zone: &str) -> Option<i32> {
    let sign = match zone.as_bytes().first() {
        Some(b'+') => 1_i32,
        Some(b'-') => -1,
        _ => return None,
    };
    let rest = zone.get(1..)?;
    let (hour, minute) = match rest.len() {
        2 => (parse_num(rest, 0, 2)?, 0),
        4 => (parse_num(rest, 0, 2)?, parse_num(rest, 2, 4)?),
        5 if rest.as_bytes().get(2) == Some(&b':') => {
            (parse_num(rest, 0, 2)?, parse_num(rest, 3, 5)?)
        }
        8 if rest.as_bytes().get(2) == Some(&b':') && rest.as_bytes().get(5) == Some(&b':') => {
            (parse_num(rest, 0, 2)?, parse_num(rest, 3, 5)?)
        }
        _ => return None,
    };
    if hour > 14 || minute > 59 {
        return None;
    }
    Some(sign * (hour * 60 + minute))
}

#[derive(Clone, Copy)]
struct Parts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millis: u32,
}

fn parse_body(body: &str) -> Option<Parts> {
    if body.len() >= 10 && body.as_bytes().get(4) == Some(&b'-') {
        let date = body.get(..10)?;
        let (year, month, day) = parse_date(date)?;
        let time = if body.len() == 10 {
            ""
        } else {
            let sep = body.as_bytes().get(10)?;
            if *sep != b'T' && *sep != b' ' {
                return None;
            }
            body.get(11..)?
        };
        let (hour, minute, second, millis) = if time.is_empty() {
            (0, 0, 0, 0)
        } else {
            parse_time(time)?
        };
        return Some(Parts {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millis,
        });
    }
    let (hour, minute, second, millis) = parse_time(body)?;
    Some(Parts {
        year: 2000,
        month: 1,
        day: 1,
        hour,
        minute,
        second,
        millis,
    })
}

fn parse_date(date: &str) -> Option<(i32, u32, u32)> {
    if date.len() != 10
        || date.as_bytes().get(4) != Some(&b'-')
        || date.as_bytes().get(7) != Some(&b'-')
    {
        return None;
    }
    let year = parse_num(date, 0, 4)?;
    let month = u32::try_from(parse_num(date, 5, 7)?).ok()?;
    let day = u32::try_from(parse_num(date, 8, 10)?).ok()?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

fn parse_time(time: &str) -> Option<(u32, u32, u32, u32)> {
    if time.len() < 5 || time.as_bytes().get(2) != Some(&b':') {
        return None;
    }
    let hour = u32::try_from(parse_num(time, 0, 2)?).ok()?;
    let minute = u32::try_from(parse_num(time, 3, 5)?).ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    if time.len() == 5 {
        return Some((hour, minute, 0, 0));
    }
    if time.as_bytes().get(5) != Some(&b':') || time.len() < 8 {
        return None;
    }
    let second = u32::try_from(parse_num(time, 6, 8)?).ok()?;
    if second > 59 {
        return None;
    }
    if time.len() == 8 {
        return Some((hour, minute, second, 0));
    }
    if time.as_bytes().get(8) != Some(&b'.') {
        return None;
    }
    let frac = time.get(9..)?;
    Some((hour, minute, second, frac_millis(frac)?))
}

fn frac_millis(frac: &str) -> Option<u32> {
    if frac.is_empty() || frac.len() > 9 || !frac.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut padded = frac.to_owned();
    while padded.len() < 3 {
        padded.push('0');
    }
    padded.get(..3)?.parse().ok()
}

fn parse_num(text: &str, start: usize, end: usize) -> Option<i32> {
    let slice = text.get(start..end)?;
    if slice.is_empty() || !slice.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    slice.parse().ok()
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(year: i32) -> bool {
    let by4 = year.rem_euclid(4) == 0;
    let by100 = year.rem_euclid(100) == 0;
    let by400 = year.rem_euclid(400) == 0;
    by4 && (!by100 || by400)
}

fn civil_to_unix_ms(parts: Parts) -> Option<i64> {
    let days = days_from_civil(parts.year, parts.month, parts.day)?;
    let hour = i64::from(parts.hour);
    let minute = i64::from(parts.minute);
    let second = i64::from(parts.second);
    let millis = i64::from(parts.millis);
    let secs = days
        .checked_mul(86_400)?
        .checked_add(hour.checked_mul(3600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    secs.checked_mul(1000)?.checked_add(millis)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let year = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = year.div_euclid(400);
    let yoe = i64::from(year.checked_sub(era.checked_mul(400)?)?);
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * month_prime + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe.div_euclid(4) - yoe.div_euclid(100) + i64::from(doy);
    Some(i64::from(era) * 146_097 + doe - 719_468)
}

fn format_parts(at: Timestamp) -> Option<String> {
    let secs = at.millis().div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
    ))
}

fn civil_from_days(days: i64) -> Option<(i32, u32, u32)> {
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = u64::try_from(z.checked_sub(era.checked_mul(146_097)?)?).ok()?;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let mut year = i32::try_from(yoe)
        .ok()?
        .checked_add(i32::try_from(era).ok()?.checked_mul(400)?)?;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year = year.checked_add(1)?;
    }
    Some((year, u32::try_from(month).ok()?, u32::try_from(day).ok()?))
}

#[cfg(test)]
mod tests {
    use super::{escape_controls, format_sqlite_utc, parse_since, rejected_since};
    use crate::chat::ChatError;
    use crate::clock::Timestamp;

    fn at(text: &str) -> Timestamp {
        parse_since(text, Timestamp::from_millis(0)).expect(text)
    }

    #[test]
    fn offsets_match_their_utc_twin() {
        let utc = at("2026-09-30T11:00:00Z");
        assert_eq!(at("2026-09-30 11:00:00"), utc);
        assert_eq!(at("2026-09-30T07:00:00-04:00"), utc);
        assert_eq!(at("2026-09-30T13:00:00+02:00"), utc);
        assert!(at("2026-09-30T11:00:00.500Z") > utc);
        assert_eq!(at("2020-01-01"), at("2020-01-01 00:00:00"));
        assert_eq!(at("1970-01-01 00:00:00").millis(), 0);
        assert_eq!(format_sqlite_utc(utc), "2026-09-30 11:00:00");
        assert_eq!(
            format_sqlite_utc(at("2026-09-30T11:00:00.500Z")),
            "2026-09-30 11:00:00"
        );
    }

    #[test]
    fn garbage_whitespace_and_now() {
        let now = Timestamp::from_millis(1_700_000_000_000);
        assert_eq!(parse_since("now", now).expect("now"), now);
        assert!(matches!(
            parse_since("garbage", now),
            Err(ChatError::UnparseableSince(_))
        ));
        assert!(matches!(
            parse_since("   ", now),
            Err(ChatError::UnparseableSince(_))
        ));
        assert!(matches!(
            parse_since("not a timestamp", now),
            Err(ChatError::UnparseableSince(_))
        ));
    }

    #[test]
    fn escape_keeps_letters_and_shows_controls() {
        assert_eq!(escape_controls("a\nb\u{1b}c"), "a\\x0ab\\x1bc");
        assert_eq!(escape_controls("a\u{9b}béc"), "a\\x9bbéc");
        assert!(!escape_controls("a\nb").contains('\n'));
        let rejected = rejected_since("a\nb\u{1b}c");
        assert!(rejected.contains("a\\x0ab\\x1bc"));
        assert!(!rejected.contains('\n'));
    }
}
