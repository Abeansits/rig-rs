// Derived from openrig packages/tui/src/input.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Terminal modes around an external attach.
//!
//! Suspend leaves the alternate screen, then disables raw mode, mouse capture and
//! bracketed paste. Resume does the reverse.

use std::io::{self, Write, stdout};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Restores and re-enters the four TUI modes.
pub trait TerminalGuard {
    /// Leave the alternate screen and disable raw mode, mouse capture and bracketed paste.
    ///
    /// # Errors
    ///
    /// Returns the terminal I/O error.
    fn suspend(&mut self) -> io::Result<()>;

    /// Enter the alternate screen and enable raw mode, mouse capture and bracketed paste.
    ///
    /// # Errors
    ///
    /// Returns the terminal I/O error.
    fn resume(&mut self) -> io::Result<()>;
}

/// Crossterm on the process stdout.
#[derive(Debug, Default)]
pub struct CrosstermGuard;

impl TerminalGuard for CrosstermGuard {
    fn suspend(&mut self) -> io::Result<()> {
        suspend_stdout()
    }

    fn resume(&mut self) -> io::Result<()> {
        resume_stdout()
    }
}

/// Suspend stdout. Also used by the panic hook, which must run before the panic is printed.
///
/// # Errors
///
/// Returns the terminal I/O error.
pub(crate) fn suspend_stdout() -> io::Result<()> {
    let mut out = stdout();
    execute!(out, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    execute!(out, DisableMouseCapture, DisableBracketedPaste)?;
    out.flush()
}

/// Resume stdout.
///
/// # Errors
///
/// Returns the terminal I/O error.
pub(crate) fn resume_stdout() -> io::Result<()> {
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    enable_raw_mode()?;
    execute!(out, EnableMouseCapture, EnableBracketedPaste)?;
    out.flush()
}

/// Install a panic hook that runs `restore` before the previous hook prints.
pub(crate) fn install_panic_hook(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// Hook that leaves the alternate screen and raw mode before the panic is printed.
pub(crate) fn install_terminal_hook() {
    install_panic_hook(|| {
        if let Err(err) = suspend_stdout() {
            tracing::error!(error = %err, "terminal restore during panic failed");
        }
    });
}
