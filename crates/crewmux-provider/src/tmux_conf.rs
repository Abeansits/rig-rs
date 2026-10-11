//! Marker-delimited managed block for `~/.tmux.conf`.
//!
//! The block never sets `window-size` or `remain-on-exit` at global scope.
//! `remain-on-exit` belongs on the session created for a rig seat.

use crate::error::ProviderError;

/// Begin and end markers for one managed region.
#[derive(Debug, Clone, Copy)]
pub struct ManagedBlock {
    /// Line that opens the block.
    pub begin: &'static str,
    /// Line that closes the block.
    pub end: &'static str,
}

impl ManagedBlock {
    /// Markers used in the user's tmux config.
    pub const TMUX: Self = Self {
        begin: "# >>> rig-rs managed >>>",
        end: "# <<< rig-rs managed <<<",
    };

    /// Collapse every managed region into one current block, or append one.
    /// Text outside the markers stays in order. A second call with the same
    /// body returns the same string. Files that use CRLF keep CRLF.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when a begin marker has no matching end.
    pub fn upsert(&self, file: &str, body: &str) -> Result<String, ProviderError> {
        let newline = newline_of(file);
        let block = render_with(self, body, newline);
        let mut out = String::new();
        let mut rest = file;
        let mut replaced = false;
        loop {
            match span(rest, self.begin, self.end) {
                Span::Absent => {
                    out.push_str(rest);
                    break;
                }
                Span::Broken => {
                    return Err(ProviderError::Parse {
                        file: std::path::PathBuf::from("tmux.conf"),
                        msg: "managed block begin marker has no end marker".to_owned(),
                    });
                }
                Span::Found { start, end } => {
                    out.push_str(rest.get(..start).unwrap_or(""));
                    if !replaced {
                        out.push_str(&block);
                        replaced = true;
                    }
                    rest = rest.get(end..).unwrap_or("");
                }
            }
        }
        if replaced {
            Ok(out)
        } else {
            Ok(append(file, &block, newline))
        }
    }

    /// Delete every managed region. Bytes outside the markers stay in place.
    #[must_use]
    pub fn remove(&self, file: &str) -> String {
        let mut current = file.to_owned();
        loop {
            let Span::Found { start, end } = span(&current, self.begin, self.end) else {
                return current;
            };
            let mut out = String::new();
            out.push_str(current.get(..start).unwrap_or(""));
            out.push_str(current.get(end..).unwrap_or(""));
            current = out;
        }
    }
}

/// Body of the tmux managed block. It sets mouse and history.
/// It does not set a global window size or a global `remain-on-exit`.
pub const TMUX_BLOCK_BODY: &str = "\
set -g mouse on
set -g history-limit 50000
";

/// `begin`, body, and `end`, each on its own line, with a trailing newline.
#[must_use]
pub fn render_tmux_block() -> String {
    render_with(&ManagedBlock::TMUX, TMUX_BLOCK_BODY, "\n")
}

/// The managed block is present and matches [`render_tmux_block`].
#[must_use]
pub fn managed_block_current(file: &str) -> bool {
    file.contains(&render_tmux_block())
}

/// A global `window-size` set to `manual` crashes tmux 3.5a.
///
/// A line matches when the command is `set`, `set-option`, `setw`, or
/// `set-window-option`, a short-flag group contains `g`, and the unquoted
/// value is `manual`. A `#` outside quotes starts a comment.
#[must_use]
pub fn global_window_size_manual(file: &str) -> bool {
    file.lines().any(line_sets_manual_window_size)
}

fn line_sets_manual_window_size(line: &str) -> bool {
    let tokens = tokenize(line);
    let mut tokens = tokens.into_iter();
    let Some(cmd) = tokens.next() else {
        return false;
    };
    if !is_set_command(&cmd) {
        return false;
    }
    let mut global = false;
    let mut option = None;
    let mut value = None;
    let mut expect_target = false;
    for token in tokens {
        if expect_target {
            expect_target = false;
            continue;
        }
        if is_short_flags(&token) {
            if token.contains('g') {
                global = true;
            }
            if token.contains('t') {
                expect_target = true;
            }
            continue;
        }
        if option.is_none() {
            option = Some(token);
        } else {
            value = Some(token);
            break;
        }
    }
    global && option.as_deref() == Some("window-size") && value.as_deref() == Some("manual")
}

fn is_set_command(cmd: &str) -> bool {
    matches!(cmd, "set" | "set-option" | "setw" | "set-window-option")
}

fn is_short_flags(token: &str) -> bool {
    let Some(flags) = token.strip_prefix('-') else {
        return false;
    };
    !flags.is_empty() && flags.chars().all(|ch| ch.is_ascii_alphabetic())
}

/// Split a config line into words. Quotes drop off the token. `#` outside
/// quotes ends the line. An unclosed quote drops that token.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(mark) = quote {
            if mark == '"' && ch == '\\' {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
                continue;
            }
            if ch == mark {
                tokens.push(std::mem::take(&mut current));
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '#' {
            break;
        }
        if ch == '"' || ch == '\'' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if quote.is_none() && !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn newline_of(file: &str) -> &'static str {
    if file.contains("\r\n") { "\r\n" } else { "\n" }
}

fn render_with(block: &ManagedBlock, body: &str, newline: &str) -> String {
    let mut out = String::new();
    out.push_str(block.begin);
    out.push_str(newline);
    out.push_str(&body.trim_end().replace('\n', newline));
    out.push_str(newline);
    out.push_str(block.end);
    out.push_str(newline);
    out
}

fn append(file: &str, block: &str, newline: &str) -> String {
    if file.is_empty() {
        return block.to_owned();
    }
    let mut out = file.to_owned();
    if !out.ends_with('\n') {
        out.push_str(newline);
    }
    out.push_str(block);
    out
}

enum Span {
    Absent,
    Broken,
    Found { start: usize, end: usize },
}

fn span(file: &str, begin: &str, end_marker: &str) -> Span {
    let Some(start) = file.find(begin) else {
        return Span::Absent;
    };
    let Some(relative) = file.get(start..).and_then(|tail| tail.find(end_marker)) else {
        return Span::Broken;
    };
    let mut end = start
        .saturating_add(relative)
        .saturating_add(end_marker.len());
    if file.as_bytes().get(end) == Some(&b'\r')
        && file.as_bytes().get(end.saturating_add(1)) == Some(&b'\n')
    {
        end = end.saturating_add(2);
    } else if file.as_bytes().get(end) == Some(&b'\n') {
        end = end.saturating_add(1);
    }
    Span::Found { start, end }
}
