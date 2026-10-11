//! Regex literals in a TypeScript test, so quotes inside them are not strings.

use super::Scan;

pub(super) fn try_skip_regex(scan: &mut Scan<'_>) -> bool {
    if scan.peek() != Some(b'/') || !regex_context(scan) {
        return false;
    }
    let next = scan
        .source
        .as_bytes()
        .get(scan.index.saturating_add(1))
        .copied();
    if matches!(next, Some(b'/' | b'*')) {
        return false;
    }
    let start = scan.index;
    scan.bump(1);
    let mut in_class = false;
    while let Some(byte) = scan.peek() {
        if byte == b'\n' {
            scan.index = start;
            return false;
        }
        if byte == b'\\' {
            scan.bump(1);
            scan.bump_char();
            continue;
        }
        if byte == b'[' && !in_class {
            in_class = true;
            scan.bump(1);
            continue;
        }
        if byte == b']' && in_class {
            in_class = false;
            scan.bump(1);
            continue;
        }
        if byte == b'/' && !in_class {
            scan.bump(1);
            while scan.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
                scan.bump(1);
            }
            return true;
        }
        scan.bump_char();
    }
    scan.index = start;
    false
}

fn regex_context(scan: &Scan<'_>) -> bool {
    let bytes = scan.source.as_bytes();
    let mut index = scan.index;
    loop {
        let Some(prev) = index.checked_sub(1) else {
            return true;
        };
        let Some(byte) = bytes.get(prev).copied() else {
            return true;
        };
        if byte.is_ascii_whitespace() {
            index = prev;
            continue;
        }
        if matches!(
            byte,
            b'(' | b'='
                | b','
                | b':'
                | b'['
                | b'!'
                | b'&'
                | b'|'
                | b'?'
                | b'{'
                | b'}'
                | b';'
                | b'~'
        ) {
            return true;
        }
        if !is_ident(byte) {
            return false;
        }
        let mut start = prev;
        while start > 0
            && bytes
                .get(start.saturating_sub(1))
                .is_some_and(|byte| is_ident(*byte))
        {
            start = start.saturating_sub(1);
        }
        return matches!(
            scan.source.get(start..=prev),
            Some("return" | "typeof" | "case" | "void" | "throw" | "delete" | "await" | "yield")
        );
    }
}

pub(super) fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}
