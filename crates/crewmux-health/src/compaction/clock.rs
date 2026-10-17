//! Injected millisecond clock and the UTC stamp the prep prompt quotes.

/// Milliseconds since the Unix epoch.
///
/// The enforcer takes this instead of [`std::time::Instant`] so a test can
/// move the clock forward by an exact duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Millis(pub u64);

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for `ms` since the Unix epoch.
#[must_use]
pub fn iso_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let millis = ms % 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let tod = secs % 86_400;
    let hour = tod / 3_600;
    let minute = (tod % 3_600) / 60;
    let second = tod % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Howard Hinnant's civil-from-days, shifted to the Unix epoch.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    let year_i = i32::try_from(year).unwrap_or(0);
    let month_u = u32::try_from(month).unwrap_or(1);
    let day_u = u32::try_from(day).unwrap_or(1);
    (year_i, month_u, day_u)
}

#[cfg(test)]
mod tests {
    use super::iso_utc;

    #[test]
    fn unix_millis_format_matches_utc() {
        assert_eq!(iso_utc(1_700_000_000_000), "2023-11-14T22:13:20.000Z");
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
    }
}
