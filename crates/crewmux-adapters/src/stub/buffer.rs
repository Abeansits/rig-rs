//! Bracketed-paste composer. A real TUI keeps the paste in a buffer: `C-u`
//! drops the current line, and Enter submits whatever remains.

/// What the stub reader saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StdinEvent {
    /// A canonical line, including its newline. Not a submit.
    Line(String),
    /// Enter completed a line that was already held. Echo it and redraw the prompt.
    Finish(String),
    /// `C-u` emptied the draft. Redraw the prompt. Do not submit.
    Redraw,
    /// A bracketed paste finished. The tty already echoed the body.
    Bracket,
    /// `C-u` left this text in the buffer.
    Partial(String),
    /// Enter submitted the buffered composer.
    Submit(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Text,
    Esc,
    Csi,
    Paste,
    PasteEsc,
    PasteCsi,
}

/// `C-g`. Drops the whole draft from any scanner state and redraws the prompt.
/// Not a tty signal, so the stub stays up.
pub(crate) const RESET_DRAFT: u8 = 0x07;

/// Byte scanner for `buffer=1`.
pub(crate) struct Scanner {
    mode: Mode,
    pending: Vec<u8>,
    paste: Vec<u8>,
    csi: Vec<u8>,
    held: String,
}

impl Scanner {
    pub(crate) fn new() -> Self {
        Self {
            mode: Mode::Text,
            pending: Vec::new(),
            paste: Vec::new(),
            csi: Vec::new(),
            held: String::new(),
        }
    }

    pub(crate) fn push(&mut self, byte: u8) -> Option<StdinEvent> {
        if byte == RESET_DRAFT {
            return Some(self.reset());
        }
        match self.mode {
            Mode::Text => self.text(byte),
            Mode::Esc => self.esc(byte, false),
            Mode::Csi => self.csi(byte, false),
            Mode::Paste => self.paste_byte(byte),
            Mode::PasteEsc => self.esc(byte, true),
            Mode::PasteCsi => self.csi(byte, true),
        }
    }

    fn text(&mut self, byte: u8) -> Option<StdinEvent> {
        if byte == 0x1b {
            self.mode = Mode::Esc;
            return None;
        }
        if byte == 0x15 {
            return Some(self.clear_line());
        }
        if byte == b'\n' && !self.held.is_empty() {
            return Some(StdinEvent::Submit(std::mem::take(&mut self.held)));
        }
        self.pending.push(byte);
        if byte != b'\n' {
            return None;
        }
        let line = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        Some(StdinEvent::Line(line))
    }

    fn reset(&mut self) -> StdinEvent {
        self.mode = Mode::Text;
        self.pending.clear();
        self.paste.clear();
        self.csi.clear();
        self.held.clear();
        StdinEvent::Redraw
    }

    fn clear_line(&mut self) -> StdinEvent {
        self.pending.clear();
        if self.held.is_empty() {
            return StdinEvent::Redraw;
        }
        if let Some(idx) = self.held.rfind('\n') {
            self.held.truncate(idx);
        } else {
            self.held.clear();
        }
        if self.held.is_empty() {
            StdinEvent::Redraw
        } else {
            StdinEvent::Partial(self.held.clone())
        }
    }

    fn esc(&mut self, byte: u8, in_paste: bool) -> Option<StdinEvent> {
        if byte == b'[' {
            self.csi.clear();
            self.mode = if in_paste { Mode::PasteCsi } else { Mode::Csi };
            return None;
        }
        self.push_literal(0x1b, in_paste);
        self.push_literal(byte, in_paste);
        self.mode = if in_paste { Mode::Paste } else { Mode::Text };
        None
    }

    fn csi(&mut self, byte: u8, in_paste: bool) -> Option<StdinEvent> {
        if byte.is_ascii_digit() || byte == b';' {
            self.csi.push(byte);
            return None;
        }
        let end = byte == b'~';
        let start_paste = end && !in_paste && self.csi.as_slice() == b"200";
        let finish_paste = end && in_paste && self.csi.as_slice() == b"201";
        self.csi.clear();
        if start_paste {
            self.paste.clear();
            self.mode = Mode::Paste;
            return None;
        }
        if finish_paste {
            self.held = String::from_utf8_lossy(&self.paste).into_owned();
            self.paste.clear();
            self.mode = Mode::Text;
            return Some(StdinEvent::Bracket);
        }
        self.mode = if in_paste { Mode::Paste } else { Mode::Text };
        None
    }

    fn paste_byte(&mut self, byte: u8) -> Option<StdinEvent> {
        if byte == 0x1b {
            self.mode = Mode::PasteEsc;
            return None;
        }
        self.paste.push(byte);
        None
    }

    fn push_literal(&mut self, byte: u8, in_paste: bool) {
        if in_paste {
            self.paste.push(byte);
        } else {
            self.pending.push(byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Scanner, StdinEvent};

    #[test]
    fn c_u_drops_one_line_and_enter_submits_the_rest() {
        let mut scan = Scanner::new();
        let bytes = b"\x1b[200~From: a\nTo: b\n---\nhi\x1b[201~";
        let mut bracket = false;
        for &byte in bytes {
            if matches!(scan.push(byte), Some(StdinEvent::Bracket)) {
                bracket = true;
            }
        }
        assert!(bracket);
        let partial = scan.push(0x15);
        assert!(
            matches!(partial, Some(StdinEvent::Partial(ref text)) if text.contains("From: a")),
            "{partial:?}"
        );
        let submit = scan.push(b'\n');
        match submit {
            Some(StdinEvent::Submit(text)) => {
                assert!(text.contains("From: a"), "{text}");
                assert!(!text.contains("hi"), "{text}");
            }
            other => panic!("enter submitted the remainder: {other:?}"),
        }
    }

    #[test]
    fn reset_drops_a_partial_paste_and_the_held_draft() {
        let mut scan = Scanner::new();
        for &byte in b"\x1b[200~line\nmore" {
            drop(scan.push(byte));
        }
        assert_eq!(scan.push(super::RESET_DRAFT), Some(StdinEvent::Redraw));
        let enter = scan.push(b'\n');
        assert!(
            !matches!(enter, Some(StdinEvent::Submit(_))),
            "reset draft must not submit: {enter:?}"
        );
    }
}
