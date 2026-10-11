//! Octal decoder for tmux control-mode `%output` payloads.
//!
//! tmux 3.5 writes each byte below ASCII space, and the backslash byte, as
//! `\ooo`. Every other byte, including DEL and UTF-8, is raw. A read may split
//! an escape, so the decoder keeps a short pending tail.

use crate::error::MuxError;

/// Incremental `%output` payload decoder.
#[derive(Debug)]
pub(crate) struct OutputDecoder {
    pending: Vec<u8>,
}

impl Default for OutputDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputDecoder {
    /// Create an empty decoder.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// `true` when no partial escape is waiting for more bytes.
    #[must_use]
    pub(crate) fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }

    /// Decode the next chunk. Incomplete trailing escapes stay pending.
    pub(crate) fn push(&mut self, input: &[u8]) -> Result<Vec<u8>, MuxError> {
        self.pending.extend_from_slice(input);
        let pending = std::mem::take(&mut self.pending);
        let mut rest = pending.as_slice();
        let mut out = Vec::new();
        loop {
            match rest.split_first() {
                None => break,
                Some((b'\\', tail)) => {
                    if let Some((digits, after)) = split_three(tail) {
                        let value = parse_octal(digits)?;
                        out.push(value);
                        rest = after;
                    } else {
                        self.pending.push(b'\\');
                        self.pending.extend_from_slice(tail);
                        break;
                    }
                }
                Some((byte, tail)) => {
                    out.push(*byte);
                    rest = tail;
                }
            }
        }
        Ok(out)
    }
}

/// Decode a complete payload, rejecting a truncated trailing escape.
#[cfg(test)]
pub(crate) fn decode_output(input: &[u8]) -> Result<Vec<u8>, MuxError> {
    let mut decoder = OutputDecoder::new();
    let out = decoder.push(input)?;
    if decoder.is_idle() {
        Ok(out)
    } else {
        Err(MuxError::Control("truncated tmux octal escape".to_owned()))
    }
}

/// Encode the way tmux 3.5 `control_append_data` does, for tests.
#[cfg(test)]
#[must_use]
pub(crate) fn encode_output(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &byte in data {
        if byte < b' ' || byte == b'\\' {
            out.extend(format!("\\{byte:03o}").into_bytes());
        } else {
            out.push(byte);
        }
    }
    out
}

fn split_three(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let digits = bytes.get(..3)?;
    let rest = bytes.get(3..)?;
    Some((digits, rest))
}

fn parse_octal(digits: &[u8]) -> Result<u8, MuxError> {
    let mut value: u16 = 0;
    if digits.is_empty() {
        return Err(MuxError::Control("empty octal escape".to_owned()));
    }
    for digit in digits {
        let number = digit
            .checked_sub(b'0')
            .ok_or_else(|| MuxError::Control("non-digit in octal escape".to_owned()))?;
        if number > 7 {
            return Err(MuxError::Control("non-octal digit in escape".to_owned()));
        }
        value = value
            .checked_mul(8)
            .and_then(|current| current.checked_add(u16::from(number)))
            .ok_or_else(|| MuxError::Control("octal escape overflow".to_owned()))?;
    }
    u8::try_from(value)
        .map_err(|_| MuxError::Control("octal escape does not fit a byte".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{OutputDecoder, decode_output, encode_output};

    #[test]
    fn round_trip_controls_utf8_and_del() {
        let sample = b"h\xc3\xa9llo\t\r\x1b[31m\\\0\x7f\xff\n\"'`$HOME";
        let decoded = decode_output(&encode_output(sample)).expect("decode");
        assert_eq!(decoded, sample);
    }

    #[test]
    fn escape_split_across_pushes() {
        let encoded = encode_output(b"\r\n");
        let mut decoder = OutputDecoder::new();
        let mut got = Vec::new();
        for byte in &encoded {
            got.extend(decoder.push(std::slice::from_ref(byte)).expect("push"));
        }
        assert!(decoder.is_idle());
        assert_eq!(got, b"\r\n");
    }

    proptest::proptest! {
        #[test]
        fn property_decoder_matches_bytes(
            data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..128usize),
            cuts in proptest::collection::vec(0usize..32, 0..6usize),
        ) {
            let encoded = encode_output(&data);
            let mut decoder = OutputDecoder::new();
            let mut got = Vec::new();
            let mut offset = 0usize;
            if encoded.is_empty() {
                got.extend(decoder.push(&[]).expect("empty"));
            }
            for cut in cuts {
                if offset >= encoded.len() {
                    break;
                }
                let remain = encoded.len() - offset;
                let step = 1 + (cut % remain);
                let end = offset + step;
                let slice = encoded.get(offset..end).expect("range");
                got.extend(decoder.push(slice).expect("push"));
                offset = end;
            }
            if let Some(tail) = encoded.get(offset..) {
                got.extend(decoder.push(tail).expect("tail"));
            }
            proptest::prop_assert!(decoder.is_idle());
            proptest::prop_assert_eq!(got, data);
        }
    }
}
