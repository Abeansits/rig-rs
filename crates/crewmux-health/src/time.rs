//! UTC timestamps with millisecond precision, without an extra time crate.

use crewmux_core::Timestamp;

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for `millis` since the Unix epoch.
#[must_use]
pub fn iso_millis(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let frac = millis.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{frac:03}Z")
}

/// ISO form of a domain timestamp.
#[must_use]
pub fn iso_timestamp(at: Timestamp) -> String {
    iso_millis(at.millis())
}

/// Howard Hinnant's `civil_from_days`, public domain.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    let year = i32::try_from(year).unwrap_or(i32::MAX);
    (
        year,
        u32::try_from(month).unwrap_or(0),
        u32::try_from(day).unwrap_or(0),
    )
}
