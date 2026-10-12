//! Small parsers for rollout timestamps and on-screen version numbers.

use std::time::{Duration, SystemTime};

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z` as a UTC [`SystemTime`].
#[must_use]
pub(super) fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = text.split_once('T')?;
    let mut dates = date.split('-');
    let year: i32 = dates.next()?.parse().ok()?;
    let month: u32 = dates.next()?.parse().ok()?;
    let day: u32 = dates.next()?.parse().ok()?;
    if dates.next().is_some() {
        return None;
    }
    let hms = time.split_once('.').map_or(time, |(clock, _)| clock);
    let mut clock = hms.split(':');
    let hour: u32 = clock.next()?.parse().ok()?;
    let minute: u32 = clock.next()?.parse().ok()?;
    let second: u32 = clock.next()?.parse().ok()?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour).checked_mul(3_600)?)?
        .checked_add(i64::from(minute).checked_mul(60)?)?
        .checked_add(i64::from(second))?;
    let secs = u64::try_from(secs).ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

/// Semver tokens (`major.minor.patch`) in order.
#[must_use]
pub(super) fn semvers(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if let Some((token, next)) = take_semver(&chars, index) {
            out.push(token);
            index = next;
        } else {
            index = index.saturating_add(1);
        }
    }
    out
}

fn take_semver(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut dots = 0u8;
    let mut saw_digit = false;
    let mut index = start;
    while let Some(ch) = chars.get(index).copied() {
        if ch.is_ascii_digit() {
            out.push(ch);
            saw_digit = true;
            index = index.saturating_add(1);
        } else if ch == '.' && saw_digit && dots < 2 {
            out.push(ch);
            dots = dots.saturating_add(1);
            saw_digit = false;
            index = index.saturating_add(1);
        } else {
            break;
        }
    }
    if dots == 2 && saw_digit {
        Some((out, index))
    } else {
        None
    }
}

/// Days since the Unix epoch. Month is 1-based.
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * month_prime + 2) / 5 + day - 1;
    let yoe_u = u32::try_from(yoe).ok()?;
    let doe = yoe_u
        .checked_mul(365)?
        .checked_add(yoe_u / 4)?
        .checked_sub(yoe_u / 100)?
        .checked_add(doy)?;
    Some(i64::from(era) * 146_097 + i64::from(doe) - 719_468)
}
