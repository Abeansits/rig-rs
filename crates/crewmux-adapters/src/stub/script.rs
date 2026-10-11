//! `RIG_STUB_SCRIPT` grammar: comma-separated `key=value` pairs.
//!
//! This is the slice 1 script, not the TypeScript JSON step list.

use crate::error::Error;

/// Parsed stub behaviour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StubScript {
    /// Delay before the composer or the gate line, in milliseconds.
    pub ready_after_ms: u64,
    /// Exit after this many milliseconds from process start.
    pub exit_after_ms: Option<u64>,
    /// Gate code to print instead of `stub> `.
    pub print_gate: Option<String>,
}

impl StubScript {
    /// Screen once the ready delay has elapsed.
    #[must_use]
    pub fn screen(&self) -> String {
        let mut text = String::from("stub-agent\n");
        if let Some(code) = &self.print_gate {
            text.push_str("stub gate: ");
            text.push_str(code);
            text.push('\n');
        } else {
            text.push_str("stub> \n");
        }
        text
    }

    /// Banner shown before [`Self::screen`] when `ready_after_ms` is non-zero.
    #[must_use]
    pub fn banner() -> &'static str {
        "stub-agent\n"
    }
}

/// Parse `ready_after_ms`, `exit_after_ms`, and `print_gate`.
///
/// An empty string is the default: ready immediately, no exit, no gate.
///
/// # Errors
///
/// Returns [`Error::Message`] for an unknown key, a missing `=`, or a bad integer.
pub fn parse_script(raw: &str) -> Result<StubScript, Error> {
    let mut script = StubScript::default();
    if raw.trim().is_empty() {
        return Ok(script);
    }
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, value)) = part.split_once('=') else {
            return Err(Error::Message(format!(
                "RIG_STUB_SCRIPT entry `{part}` needs key=value"
            )));
        };
        match key.trim() {
            "ready_after_ms" => script.ready_after_ms = parse_ms(value)?,
            "exit_after_ms" => script.exit_after_ms = Some(parse_ms(value)?),
            "print_gate" => script.print_gate = Some(value.trim().to_owned()),
            other => {
                return Err(Error::Message(format!(
                    "unknown RIG_STUB_SCRIPT key `{other}`"
                )));
            }
        }
    }
    Ok(script)
}

fn parse_ms(value: &str) -> Result<u64, Error> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| Error::Message(format!("invalid millisecond value `{value}`")))
}
