//! Marker-delimited managed block for `~/.tmux.conf`.
//!
//! The block never sets `window-size` at global scope.

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

    /// Replace the managed region, or append it. A second call with the same body
    /// returns the same string.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when a begin marker has no matching end.
    pub fn upsert(&self, file: &str, body: &str) -> Result<String, ProviderError> {
        let block = render(self, body);
        match span(file, self.begin, self.end) {
            Span::Absent => Ok(append(file, &block)),
            Span::Found { start, end } => {
                let mut out = String::new();
                out.push_str(file.get(..start).unwrap_or(""));
                out.push_str(&block);
                out.push_str(file.get(end..).unwrap_or(""));
                Ok(out)
            }
            Span::Broken => Err(ProviderError::Parse {
                file: std::path::PathBuf::from("tmux.conf"),
                msg: "managed block begin marker has no end marker".to_owned(),
            }),
        }
    }

    /// Delete the managed region. Bytes outside the markers stay in place.
    #[must_use]
    pub fn remove(&self, file: &str) -> String {
        let Span::Found { start, end } = span(file, self.begin, self.end) else {
            return file.to_owned();
        };
        let mut out = String::new();
        out.push_str(file.get(..start).unwrap_or(""));
        out.push_str(file.get(end..).unwrap_or(""));
        out
    }
}

/// Body of the tmux managed block. It sets mouse, history, and remain-on-exit.
/// It does not set a global window size.
pub const TMUX_BLOCK_BODY: &str = "\
set -g mouse on
set -g history-limit 50000
set -g remain-on-exit on
";

/// `begin`, body, and `end`, each on its own line, with a trailing newline.
#[must_use]
pub fn render_tmux_block() -> String {
    render(&ManagedBlock::TMUX, TMUX_BLOCK_BODY)
}

/// The managed block is present and matches [`render_tmux_block`].
#[must_use]
pub fn managed_block_current(file: &str) -> bool {
    file.contains(&render_tmux_block())
}

/// A global `window-size` set to `manual` crashes tmux 3.5a.
#[must_use]
pub fn global_window_size_manual(file: &str) -> bool {
    file.lines().any(line_sets_manual_window_size)
}

fn line_sets_manual_window_size(line: &str) -> bool {
    let sets_size = line.contains("window-size");
    let global = line.contains("-g");
    let manual = line.split_whitespace().any(|word| word == "manual");
    sets_size && global && manual
}

fn render(block: &ManagedBlock, body: &str) -> String {
    let mut out = String::new();
    out.push_str(block.begin);
    out.push('\n');
    out.push_str(body.trim_end());
    out.push('\n');
    out.push_str(block.end);
    out.push('\n');
    out
}

fn append(file: &str, block: &str) -> String {
    if file.is_empty() {
        return block.to_owned();
    }
    let mut out = file.to_owned();
    if !out.ends_with('\n') {
        out.push('\n');
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
    if file.as_bytes().get(end) == Some(&b'\n') {
        end = end.saturating_add(1);
    }
    Span::Found { start, end }
}
