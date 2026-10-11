//! Case titles from a pinned TypeScript checkout.

mod lex;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::types::LedgerRow;
use super::validate::split_case;

/// Case rows whose names are not `it` / `test` titles in `openrig`.
#[must_use]
pub fn check_case_titles(rows: &[LedgerRow], openrig: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let mut cache = BTreeMap::<String, Result<BTreeSet<String>, String>>::new();
    for (index, row) in rows.iter().enumerate() {
        let Some((parent, case_name)) = split_case(&row.ts_path) else {
            continue;
        };
        let line_no = index.saturating_add(2);
        let titles = cache
            .entry(parent.to_owned())
            .or_insert_with(|| load_titles(openrig, parent));
        match titles {
            Err(msg) => errors.push(format!("line {line_no}: {msg}")),
            Ok(found) if !found.contains(case_name) => errors.push(format!(
                "line {line_no}: case name `{case_name}` is not an it/test title in {parent}"
            )),
            Ok(_) => {}
        }
    }
    errors
}

fn load_titles(openrig: &Path, parent: &str) -> Result<BTreeSet<String>, String> {
    let path = openrig.join(parent);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("cannot read {} ({err})", path.display()))?;
    Ok(case_titles(&text))
}

fn case_titles(source: &str) -> BTreeSet<String> {
    let mut titles = BTreeSet::new();
    let mut scan = Scan { source, index: 0 };
    while scan.index < scan.source.len() {
        let before = scan.index;
        if let Some(title) = scan.next_title() {
            titles.insert(title);
        }
        if scan.index == before {
            scan.bump_char();
        }
    }
    titles
}

struct Scan<'a> {
    source: &'a str,
    index: usize,
}

enum Call {
    Direct,
    Each,
}

impl Scan<'_> {
    fn next_title(&mut self) -> Option<String> {
        if self.consume("//") {
            self.skip_line();
            return None;
        }
        if self.consume("/*") {
            self.skip_block();
            return None;
        }
        if lex::try_skip_regex(self) {
            return None;
        }
        if self.opens_quote() {
            self.skip_quoted();
            return None;
        }
        if self.consume("`") {
            self.skip_template();
            return None;
        }
        self.call_title()
    }

    fn call_title(&mut self) -> Option<String> {
        if !self.at_boundary() {
            return None;
        }
        if !(self.consume_word("it") || self.consume_word("test")) {
            return None;
        }
        self.skip_trivia();
        let call = if self.consume_word(".each") {
            Call::Each
        } else {
            if self.consume_word(".skip") || self.consume_word(".only") {
                self.skip_trivia();
            }
            Call::Direct
        };
        match call {
            Call::Each => self.each_title(),
            Call::Direct => self.title_after_paren(),
        }
    }

    fn each_title(&mut self) -> Option<String> {
        self.skip_type_args();
        self.skip_trivia();
        if !self.consume("(") {
            return None;
        }
        self.skip_balanced()?;
        self.skip_trivia();
        self.title_after_paren()
    }

    fn title_after_paren(&mut self) -> Option<String> {
        self.skip_trivia();
        if !self.consume("(") {
            return None;
        }
        self.skip_trivia();
        if self.opens_quote() {
            self.read_quoted()
        } else if self.consume("`") {
            self.read_template_title()
        } else {
            None
        }
    }

    fn read_template_title(&mut self) -> Option<String> {
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            if byte == b'\\' {
                self.bump(1);
                if let Some(ch) = self.pop_char() {
                    out.push(ch);
                }
                continue;
            }
            if byte == b'`' {
                self.bump(1);
                return Some(normalize_template(&out));
            }
            if self.consume("${") {
                self.skip_braces()?;
                continue;
            }
            out.push(self.pop_char()?);
        }
        None
    }

    fn read_quoted(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.bump(1);
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            if byte == b'\\' {
                self.bump(1);
                out.push(self.pop_char()?);
                continue;
            }
            if byte == quote {
                self.bump(1);
                return Some(out);
            }
            out.push(self.pop_char()?);
        }
        None
    }

    fn skip_quoted(&mut self) {
        drop(self.read_quoted());
    }

    fn skip_template(&mut self) {
        while let Some(byte) = self.peek() {
            if byte == b'\\' {
                self.bump(1);
                self.bump_char();
                continue;
            }
            if byte == b'`' {
                self.bump(1);
                return;
            }
            if self.consume("${") {
                if self.skip_braces().is_none() {
                    return;
                }
                continue;
            }
            self.bump_char();
        }
    }

    fn skip_balanced(&mut self) -> Option<()> {
        let mut depth = 1i32;
        while depth > 0 {
            if self.consume("//") {
                self.skip_line();
                continue;
            }
            if self.consume("/*") {
                self.skip_block();
                continue;
            }
            if lex::try_skip_regex(self) {
                continue;
            }
            if self.opens_quote() {
                self.skip_quoted();
                continue;
            }
            if self.consume("`") {
                self.skip_template();
                continue;
            }
            match self.peek() {
                Some(b'(') => {
                    depth = depth.saturating_add(1);
                    self.bump(1);
                }
                Some(b')') => {
                    depth -= 1;
                    self.bump(1);
                }
                Some(_) => self.bump_char(),
                None => return None,
            }
        }
        Some(())
    }

    fn skip_braces(&mut self) -> Option<()> {
        let mut depth = 1i32;
        while depth > 0 {
            if lex::try_skip_regex(self) {
                continue;
            }
            if self.opens_quote() {
                self.skip_quoted();
                continue;
            }
            if self.consume("`") {
                self.skip_template();
                continue;
            }
            match self.peek() {
                Some(b'{') => {
                    depth = depth.saturating_add(1);
                    self.bump(1);
                }
                Some(b'}') => {
                    depth -= 1;
                    self.bump(1);
                }
                Some(_) => self.bump_char(),
                None => return None,
            }
        }
        Some(())
    }

    fn skip_type_args(&mut self) {
        self.skip_trivia();
        if !self.consume("<") {
            return;
        }
        let mut depth = 1i32;
        while depth > 0 {
            match self.peek() {
                Some(b'<') => {
                    depth = depth.saturating_add(1);
                    self.bump(1);
                }
                Some(b'>') => {
                    depth -= 1;
                    self.bump(1);
                }
                Some(_) => self.bump_char(),
                None => return,
            }
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            if self.consume("//") {
                self.skip_line();
                continue;
            }
            if self.consume("/*") {
                self.skip_block();
                continue;
            }
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.bump(1),
                _ => return,
            }
        }
    }

    fn skip_line(&mut self) {
        while let Some(byte) = self.peek() {
            self.bump(1);
            if byte == b'\n' {
                return;
            }
        }
    }

    fn skip_block(&mut self) {
        while self.peek().is_some() {
            if self.consume("*/") {
                return;
            }
            self.bump_char();
        }
    }

    fn at_boundary(&self) -> bool {
        self.prev().is_none_or(|byte| !lex::is_ident(byte))
    }

    fn consume_word(&mut self, word: &str) -> bool {
        let rest = self.rest();
        let Some(tail) = rest.strip_prefix(word) else {
            return false;
        };
        if tail.starts_with(is_ident_char) {
            return false;
        }
        self.bump(word.len());
        true
    }

    fn opens_quote(&self) -> bool {
        matches!(self.peek(), Some(b'"' | b'\''))
    }

    fn consume(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.bump(token.len());
            true
        } else {
            false
        }
    }

    fn pop_char(&mut self) -> Option<char> {
        let ch = self.rest().chars().next()?;
        self.bump(ch.len_utf8());
        Some(ch)
    }

    fn bump_char(&mut self) {
        if self.pop_char().is_none() {
            self.index = self.source.len();
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.index).copied()
    }

    fn prev(&self) -> Option<u8> {
        self.index
            .checked_sub(1)
            .and_then(|index| self.source.as_bytes().get(index).copied())
    }

    fn rest(&self) -> &str {
        self.source.get(self.index..).unwrap_or("")
    }

    fn bump(&mut self, bytes: usize) {
        self.index = self.index.saturating_add(bytes);
    }
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

fn normalize_template(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed.strip_prefix(": ").unwrap_or(trimmed).to_owned()
}
