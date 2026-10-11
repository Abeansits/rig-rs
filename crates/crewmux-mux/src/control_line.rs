//! Control-mode line classification.
//!
//! Notifications are bytes, not UTF-8: tmux leaves DEL and high bytes raw.
//! `%output` payloads stay escaped until [`crate::decode`].

/// One control-mode notification.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// `%output` or `%extended-output` payload, still escaped.
    Output { pane_id: String, data: Vec<u8> },
    /// `%window-close` or `%unlinked-window-close`.
    WindowClose { window_id: String },
    /// `%paste-buffer-changed <name>`.
    PasteBuffer { name: String },
    /// `%window-renamed` / `%unlinked-window-renamed` for a `rig-dead-%N` name.
    ///
    /// tmux 3.3a does not emit `%paste-buffer-changed`. The death hook renames the
    /// bell window. The driver ignores the rename unless `window_id` is that bell.
    WindowRenamed { window_id: String, pane_id: String },
    /// Backstop subscription reported `#{pane_dead}` = 1.
    Subscription { pane_id: String },
    /// `%sessions-changed`.
    SessionsChanged,
    /// `%session-changed`.
    SessionChanged,
    /// `%exit`.
    Exit,
    /// A line the driver does not act on.
    Ignore,
}

/// Split a byte stream on `\n`, keeping a partial trailing line.
#[derive(Debug, Default)]
pub(crate) struct LineBuf {
    pending: Vec<u8>,
}

impl LineBuf {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(pos) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut rest = self.pending.split_off(pos);
            if rest.first() == Some(&b'\n') {
                rest.remove(0);
            }
            if self.pending.last() == Some(&b'\r') {
                self.pending.pop();
            }
            lines.push(std::mem::take(&mut self.pending));
            self.pending = rest;
        }
        lines
    }
}

/// Classify one control-mode line.
pub(crate) fn classify(line: &[u8]) -> Action {
    if let Some(rest) = line.strip_prefix(b"%output ") {
        return output_action(rest);
    }
    if let Some(rest) = line.strip_prefix(b"%extended-output ") {
        return extended_action(rest);
    }
    if let Some(rest) = line.strip_prefix(b"%window-close ") {
        return window_close(rest);
    }
    if let Some(rest) = line.strip_prefix(b"%unlinked-window-close ") {
        return window_close(rest);
    }
    if let Some(rest) = line.strip_prefix(b"%paste-buffer-changed ") {
        return Action::PasteBuffer {
            name: lossy_trim(rest),
        };
    }
    if let Some(rest) = line.strip_prefix(b"%window-renamed ") {
        return renamed_action(rest);
    }
    if let Some(rest) = line.strip_prefix(b"%unlinked-window-renamed ") {
        return renamed_action(rest);
    }
    if let Some(rest) = line.strip_prefix(b"%subscription-changed ") {
        return subscription_action(rest);
    }
    if line.starts_with(b"%sessions-changed") {
        return Action::SessionsChanged;
    }
    if line.starts_with(b"%session-changed") {
        return Action::SessionChanged;
    }
    if line.starts_with(b"%exit") {
        return Action::Exit;
    }
    Action::Ignore
}

fn output_action(rest: &[u8]) -> Action {
    let Some((pane_id, data)) = split_token(rest) else {
        return Action::Ignore;
    };
    Action::Output {
        pane_id: pane_id.to_owned(),
        data: data.to_vec(),
    }
}

fn extended_action(rest: &[u8]) -> Action {
    const MARK: &[u8] = b" : ";
    let Some((pane_id, after)) = split_token(rest) else {
        return Action::Ignore;
    };
    let Some(pos) = find_bytes(after, MARK) else {
        return Action::Ignore;
    };
    let data = after.get(pos + MARK.len()..).unwrap_or_default().to_vec();
    Action::Output {
        pane_id: pane_id.to_owned(),
        data,
    }
}

fn renamed_action(rest: &[u8]) -> Action {
    let text = lossy_trim(rest);
    let Some((window_id, name)) = text.split_once(' ') else {
        return Action::Ignore;
    };
    let window_id = window_id.trim();
    if window_id.is_empty() {
        return Action::Ignore;
    }
    match pane_from_dead_buffer(name.trim()) {
        Some(pane_id) => Action::WindowRenamed {
            window_id: window_id.to_owned(),
            pane_id,
        },
        None => Action::Ignore,
    }
}

fn window_close(rest: &[u8]) -> Action {
    Action::WindowClose {
        window_id: lossy_trim(rest),
    }
}

fn subscription_action(rest: &[u8]) -> Action {
    let text = String::from_utf8_lossy(rest);
    let name = text.split_whitespace().next().unwrap_or("");
    if name != "crewmux-dead" {
        return Action::Ignore;
    }
    let value = text.rsplit(':').next().map_or("", str::trim);
    if value == "1"
        && let Some(pane_id) = find_pane_id(&text)
    {
        return Action::Subscription { pane_id };
    }
    Action::Ignore
}

fn split_token(bytes: &[u8]) -> Option<(&str, &[u8])> {
    if let Some(space) = bytes.iter().position(|byte| *byte == b' ') {
        let token = std::str::from_utf8(bytes.get(..space)?).ok()?;
        let rest = bytes.get(space + 1..).unwrap_or_default();
        return Some((token, rest));
    }
    let token = std::str::from_utf8(bytes).ok()?;
    if token.is_empty() {
        None
    } else {
        Some((token, b""))
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    for index in 0..=last {
        if hay.get(index..index + needle.len()) == Some(needle) {
            return Some(index);
        }
    }
    None
}

fn find_pane_id(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes.get(index) == Some(&b'%') {
            let start = index;
            index += 1;
            let digits_at = index;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
            if index > digits_at {
                return text.get(start..index).map(str::to_owned);
            }
        } else {
            index += 1;
        }
    }
    None
}

fn lossy_trim(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

/// Pane id carried by a doorbell buffer name `rig-dead-%N`.
pub(crate) fn pane_from_dead_buffer(name: &str) -> Option<String> {
    let rest = name.trim().strip_prefix("rig-dead-")?;
    if crate::argv::is_pane_id(rest) {
        Some(rest.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, LineBuf, classify, pane_from_dead_buffer};

    #[test]
    fn live_notifications_match_tmux_3_5() {
        assert!(matches!(
            classify(b"%subscription-changed crewmux-dead $0 @1 1 %1 : 1"),
            Action::Subscription { ref pane_id } if pane_id == "%1"
        ));
        assert_eq!(
            classify(b"%subscription-changed crewmux-dead $0 @0 0 %0 : 0"),
            Action::Ignore
        );
        assert_eq!(
            classify(b"%paste-buffer-changed rig-dead-%1"),
            Action::PasteBuffer {
                name: "rig-dead-%1".to_owned()
            }
        );
        assert_eq!(pane_from_dead_buffer("rig-dead-%1").as_deref(), Some("%1"));
        assert_eq!(
            classify(b"%unlinked-window-close @1"),
            Action::WindowClose {
                window_id: "@1".to_owned()
            }
        );
        assert_eq!(classify(b"%sessions-changed"), Action::SessionsChanged);
        assert_eq!(
            classify(b"%session-changed $0 crewmux-control"),
            Action::SessionChanged
        );
        assert_eq!(classify(b"%exit"), Action::Exit);
        assert!(matches!(
            classify(br"%output %1 hi\015"),
            Action::Output { ref pane_id, ref data } if pane_id == "%1" && data == br"hi\015"
        ));
        assert!(matches!(
            classify(b"%extended-output %2 10 : z"),
            Action::Output { ref pane_id, ref data } if pane_id == "%2" && data == b"z"
        ));
    }

    #[test]
    fn window_renames_carry_the_doorbell_pane() {
        assert!(matches!(
            classify(b"%window-renamed @4 rig-dead-%2"),
            Action::WindowRenamed { ref window_id, ref pane_id }
                if window_id == "@4" && pane_id == "%2"
        ));
        assert!(matches!(
            classify(b"%unlinked-window-renamed @4 rig-dead-%3"),
            Action::WindowRenamed { ref window_id, ref pane_id }
                if window_id == "@4" && pane_id == "%3"
        ));
        assert_eq!(classify(b"%window-renamed @4 crewmux-bell"), Action::Ignore);
        assert_eq!(classify(b"%window-renamed rig-dead-%2"), Action::Ignore);
    }

    #[test]
    fn lines_split_across_reads() {
        let mut buf = LineBuf::default();
        assert_eq!(buf.push(b"%output %1 hi"), Vec::<Vec<u8>>::new());
        let lines = buf.push(b"!\n%exit\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines.first().map(Vec::as_slice),
            Some(b"%output %1 hi!".as_slice())
        );
        assert_eq!(
            classify(lines.get(1).map(Vec::as_slice).unwrap_or_default()),
            Action::Exit
        );
    }
}
