//! Codex seat vocabulary: login kind, approval flag, and rollout thread id.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::TypeError;

/// Why a Codex seat is waiting for a human to log in.
///
/// The daemon never drives the login. The remedy is `codex login` in a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginKind {
    /// Sign-in menu: the hosted account row and "Provide your own API key".
    Menu,
    /// "Finish signing in via your browser".
    ChatGptBrowser,
    /// "Enter this one-time code".
    DeviceCode,
    /// "Paste or type your API key".
    ApiKeyEntry,
    /// `codex login status` exited 1 with "Not logged in".
    PreflightNotLoggedIn,
}

impl LoginKind {
    /// Wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Menu => "menu",
            Self::ChatGptBrowser => "chatgpt_browser",
            Self::DeviceCode => "device_code",
            Self::ApiKeyEntry => "api_key_entry",
            Self::PreflightNotLoggedIn => "preflight_not_logged_in",
        }
    }

    /// Parse a wire token. Unknown text is [`Menu`](Self::Menu) so a stored row still loads.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "chatgpt_browser" => Self::ChatGptBrowser,
            "device_code" => Self::DeviceCode,
            "api_key_entry" => Self::ApiKeyEntry,
            "preflight_not_logged_in" => Self::PreflightNotLoggedIn,
            _ => Self::Menu,
        }
    }
}

impl fmt::Display for LoginKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Codex `-a` approval policy.
///
/// [`Never`](Self::Never) is not passed through. Seats launch with `on-request`
/// instead, because `-a never` refuses MCP calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodexApproval {
    /// Stored as a request. Launch uses `on-request`.
    Never,
    /// `-a on-request`. The seat default.
    OnRequest,
    /// `-a untrusted`.
    Untrusted,
}

impl CodexApproval {
    /// Parse a spec token. Anything other than `never` or `untrusted` is `on-request`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "never" => Self::Never,
            "untrusted" => Self::Untrusted,
            _ => Self::OnRequest,
        }
    }

    /// The `-a` value actually placed on the argv. Never `never`.
    #[must_use]
    pub const fn launch_arg(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnRequest | Self::Never => "on-request",
        }
    }
}

/// Codex rollout thread id (`session_meta.payload.id`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadId(String);

impl ThreadId {
    /// Reject an empty id, a token over 200 characters, or anything outside
    /// `[A-Za-z0-9._-]`. A `..` segment is rejected. The error never echoes `text`.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Id`] when `text` is empty, too long, or has
    /// disallowed characters. Surrounding whitespace is trimmed first.
    pub fn new(text: &str) -> Result<Self, TypeError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(TypeError::Id("thread id is empty".to_owned()));
        }
        if text.chars().count() > 200 {
            return Err(TypeError::Id("thread id is too long".to_owned()));
        }
        let safe = text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
        if !safe || text.contains("..") {
            return Err(TypeError::Id(
                "thread id contains disallowed characters".to_owned(),
            ));
        }
        Ok(Self(text.to_owned()))
    }

    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadId;

    #[test]
    fn accepts_a_trimmed_codex_token() {
        let id = ThreadId::new("  0199abcd_thread.id  ").expect("id");
        assert_eq!(id.as_str(), "0199abcd_thread.id");
    }

    #[test]
    fn rejects_a_token_without_echoing_it() {
        let secret = "secret/../token";
        let err = ThreadId::new(secret).expect_err("bad");
        assert!(!err.to_string().contains(secret), "{err}");
        assert!(err.to_string().contains("disallowed"), "{err}");
        let long = "a".repeat(201);
        let err = ThreadId::new(&long).expect_err("long");
        assert!(!err.to_string().contains(&long), "{err}");
        assert!(err.to_string().contains("too long"), "{err}");
        let err = ThreadId::new("   ").expect_err("empty");
        assert!(err.to_string().contains("empty"), "{err}");
        assert!(ThreadId::new(&"a".repeat(200)).is_ok());
    }
}
