// Derived from openrig packages/daemon/src/domain/context-packs/token-estimate.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `ceil(utf8_bytes / 4)`, the TypeScript token estimate.

/// Token estimate for a byte length.
#[must_use]
pub fn estimate_tokens_from_bytes(bytes: u64) -> u32 {
    let ceil = bytes.saturating_add(3) / 4;
    u32::try_from(ceil).unwrap_or(u32::MAX)
}

/// Token estimate for UTF-8 text.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    estimate_tokens_from_bytes(u64::try_from(text.len()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::estimate_tokens_from_bytes;

    #[test]
    fn ceil_divides_by_four() {
        assert_eq!(estimate_tokens_from_bytes(0), 0);
        assert_eq!(estimate_tokens_from_bytes(1), 1);
        assert_eq!(estimate_tokens_from_bytes(4), 1);
        assert_eq!(estimate_tokens_from_bytes(5), 2);
    }
}
