//! Stable screen hash. `std` `DefaultHasher` is keyed per process, so it cannot
//! back the inconclusive-budget comparison in later slices.

/// FNV-1a 64-bit of the captured text and cursor.
#[must_use]
pub(crate) fn screen_hash(text: &str, cursor: (u16, u16)) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= u64::from(cursor.0);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash ^= u64::from(cursor.1);
    hash.wrapping_mul(0x0000_0100_0000_01b3)
}

#[cfg(test)]
mod tests {
    use super::screen_hash;

    #[test]
    fn unchanged_screen_keeps_its_hash() {
        let left = screen_hash("ready\n", (1, 2));
        let right = screen_hash("ready\n", (1, 2));
        assert_eq!(left, right);
        assert_ne!(left, screen_hash("ready!\n", (1, 2)));
        assert_ne!(left, screen_hash("ready\n", (2, 2)));
    }
}
