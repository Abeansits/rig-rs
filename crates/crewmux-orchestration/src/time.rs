//! UTC timestamps in the TypeScript `toISOString` shape.

use crewmux_core::Timestamp;

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` from Unix milliseconds.
#[must_use]
pub(crate) fn iso8601(stamp: Timestamp) -> String {
    let millis = stamp.millis();
    let secs = millis.div_euclid(1000);
    let ms = millis.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let hour = tod.div_euclid(3600);
    let minute = tod.rem_euclid(3600).div_euclid(60);
    let second = tod.rem_euclid(60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{ms:03}Z")
}

/// Howard Hinnant's `civil_from_days`. `days` is days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, u32::try_from(month).unwrap_or(1), u32::try_from(day).unwrap_or(1))
}

/// Lower-case hex, no separators.
#[must_use]
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const NIBBLE: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let value = u32::from(*byte);
        let hi = usize::try_from(value >> 4).unwrap_or(0);
        let lo = usize::try_from(value & 0x0f).unwrap_or(0);
        out.push(*NIBBLE.get(hi).unwrap_or(&'0'));
        out.push(*NIBBLE.get(lo).unwrap_or(&'0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_iso() {
        assert_eq!(iso8601(Timestamp::from_millis(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso8601(Timestamp::from_millis(1_500_000_000_123)),
            "2017-07-14T02:40:00.123Z"
        );
    }
}
