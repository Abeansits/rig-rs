// Derived from openrig packages/tui/src/input.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! SGR mouse decode and the primary-button dispatch rules.

/// One decoded mouse event. Button `0` is a primary press. `64` and `65` are wheel notches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedMouse {
    /// Button bits. Clicks are masked with `& 3`. Wheels keep the full code.
    pub button: u16,
    /// 1-based column.
    pub x: u16,
    /// 1-based row.
    pub y: u16,
}

/// Wheel routing. Clicks stay on the hit map in the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// Move the explorer selection.
    Select {
        /// `3` or `-3`.
        delta: i16,
        /// How many explorer rows the caller reported.
        row_count: u16,
    },
    /// Scroll the content pane.
    ContentScroll {
        /// `3` or `-3`.
        delta: i16,
    },
}

/// Hit kinds the primary-button dispatcher knows about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HitAction {
    /// Open a view.
    Open {
        /// View identity.
        view: String,
    },
    /// Startup overlay. Only the primary button forwards it.
    Startup,
    /// Copy a labelled value.
    Copy {
        /// Label.
        label: String,
        /// Value.
        value: String,
    },
}

/// What the primary-button dispatcher invokes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dispatch {
    /// `perform` was called.
    Perform(Perform),
    /// Startup saw a primary press.
    StartupKey,
    /// The button does not activate the hit.
    None,
}

/// Argument passed to `perform`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Perform {
    /// The open action.
    Open {
        /// View identity.
        view: String,
    },
    /// The copy action.
    Copy {
        /// Label.
        label: String,
        /// Value.
        value: String,
    },
    /// Wheel routing.
    Select {
        /// `3` or `-3`.
        delta: i16,
        /// Explorer row count.
        row_count: u16,
    },
}

/// Bytes a terminal emits for a primary click at `(x, y)`, including the release.
#[must_use]
pub fn sgr_click(x: u16, y: u16) -> String {
    format!("\u{1b}[<0;{x};{y}M\u{1b}[<0;{x};{y}m")
}

/// Decode SGR mouse presses. Releases (`m`) and non-mouse bytes are dropped.
#[must_use]
pub fn decode_mice(text: &str) -> Vec<DecodedMouse> {
    let mut events = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("\u{1b}[<") {
        rest = rest.get(start..).unwrap_or(rest);
        match take_sgr(rest) {
            Some((maybe, consumed)) => {
                if let Some(event) = maybe {
                    events.push(event);
                }
                rest = rest.get(consumed..).unwrap_or("");
            }
            None => rest = rest.get(1..).unwrap_or(""),
        }
    }
    events
}

/// Wheel notches become a pane action. Other buttons return `None` so the caller hit-tests.
#[must_use]
pub fn resolve_mouse_action(
    button: u16,
    x: u16,
    explorer_width: u16,
    row_count: u16,
) -> Option<MouseAction> {
    if button & 64 == 0 {
        return None;
    }
    let delta: i16 = if button & 1 != 0 { 3 } else { -3 };
    if x <= explorer_width {
        Some(MouseAction::Select { delta, row_count })
    } else {
        Some(MouseAction::ContentScroll { delta })
    }
}

/// Dispatch one SGR press the way the TypeScript input executor does.
#[must_use]
pub fn dispatch_sgr(
    bytes: &str,
    hit: &HitAction,
    startup_open: bool,
    explorer_width: u16,
    row_count: u16,
) -> Dispatch {
    let Some(mouse) = decode_mice(bytes).into_iter().next() else {
        return Dispatch::None;
    };
    if let Some(MouseAction::Select { delta, row_count }) =
        resolve_mouse_action(mouse.button, mouse.x, explorer_width, row_count)
    {
        return Dispatch::Perform(Perform::Select { delta, row_count });
    }
    if mouse.button != 0 {
        return Dispatch::None;
    }
    match hit {
        HitAction::Startup if startup_open => Dispatch::StartupKey,
        HitAction::Startup => Dispatch::None,
        HitAction::Open { view } => Dispatch::Perform(Perform::Open { view: view.clone() }),
        HitAction::Copy { label, value } => Dispatch::Perform(Perform::Copy {
            label: label.clone(),
            value: value.clone(),
        }),
    }
}

fn take_sgr(text: &str) -> Option<(Option<DecodedMouse>, usize)> {
    let rest = text.strip_prefix("\u{1b}[<")?;
    let (button, rest) = take_u16(rest)?;
    let rest = rest.strip_prefix(';')?;
    let (x, rest) = take_u16(rest)?;
    let rest = rest.strip_prefix(';')?;
    let (y, rest) = take_u16(rest)?;
    let marker = rest.chars().next()?;
    if marker != 'M' && marker != 'm' {
        return None;
    }
    let consumed = text
        .len()
        .checked_sub(rest.len())
        .and_then(|prefix| prefix.checked_add(marker.len_utf8()))?;
    let event = if marker == 'M' {
        classify(button, x, y)
    } else {
        None
    };
    Some((event, consumed))
}

fn classify(button: u16, x: u16, y: u16) -> Option<DecodedMouse> {
    if button < 32 && (button & 3) != 3 {
        Some(DecodedMouse {
            button: button & 3,
            x,
            y,
        })
    } else if button & 64 != 0 {
        Some(DecodedMouse { button, x, y })
    } else {
        None
    }
}

fn take_u16(text: &str) -> Option<(u16, &str)> {
    let count = text.chars().take_while(char::is_ascii_digit).count();
    if count == 0 {
        return None;
    }
    let (digits, rest) = text.split_at_checked(count)?;
    let value = digits.parse().ok()?;
    Some((value, rest))
}
