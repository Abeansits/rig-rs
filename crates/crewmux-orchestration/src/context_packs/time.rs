//! ISO-8601 UTC timestamps without an extra time crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// `Date.toISOString` shape: `YYYY-MM-DDTHH:MM:SS.mmmZ`.
#[must_use]
pub(crate) fn iso_from_system(time: SystemTime) -> String {
    let millis = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0,
    };
    let millis = u64::try_from(millis).unwrap_or(u64::MAX);
    iso_from_millis(millis)
}

pub(crate) fn iso_from_millis(millis: u64) -> String {
    let secs = millis / 1000;
    let ms = millis % 1000;
    let days = secs / 86_400;
    let sod = secs % 86_400;
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;
    let (year, month, day) = civil(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{ms:03}Z")
}

fn civil(days_u: u64) -> (i32, u32, u32) {
    let days = i64::try_from(days_u).unwrap_or(i64::MAX);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = i32::try_from(if month <= 2 { y + 1 } else { y }).unwrap_or(1970);
    (year, month, day)
}
