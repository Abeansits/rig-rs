//! sha256 helpers for summaries and instruction fingerprints.

use sha2::{Digest, Sha256};

/// 32-byte sha256 of `bytes`.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    for (index, byte) in digest.iter().enumerate() {
        if let Some(slot) = out.get_mut(index) {
            *slot = *byte;
        }
    }
    out
}

/// Lower-case hex of [`sha256`].
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&sha256(bytes))
}

/// Lower-case hex.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    HEX.get(usize::from(nibble))
        .copied()
        .map_or('0', char::from)
}
