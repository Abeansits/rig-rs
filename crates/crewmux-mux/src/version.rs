//! tmux version parsing and the 3.3 minimum gate.

use std::fmt;

use crate::error::MuxError;

/// A tmux release parsed from `tmux -V`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// Major component.
    pub major: u32,
    /// Minor component.
    pub minor: u32,
    /// Trailing letter such as `a` in `3.5a`. Empty when absent.
    pub suffix: String,
    /// Set for development builds reported as `next-X.Y`.
    pub next: bool,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.next {
            write!(f, "next-")?;
        }
        write!(f, "{}.{}{}", self.major, self.minor, self.suffix)
    }
}

impl Version {
    /// Oldest tmux this crate will connect to.
    #[must_use]
    pub const fn minimum() -> Self {
        Self {
            major: 3,
            minor: 3,
            suffix: String::new(),
            next: false,
        }
    }

    /// `true` when `(major, minor)` is at least 3.3.
    ///
    /// A letter suffix does not lower the release. `next-X.Y` is treated as X.Y.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        (self.major, self.minor) >= (Self::minimum().major, Self::minimum().minor)
    }
}

/// Parse `tmux -V` output such as `tmux 3.5a` or `tmux next-3.6`.
pub fn parse_version(raw: &str) -> Result<Version, MuxError> {
    let text = raw.trim();
    let rest = text
        .strip_prefix("tmux ")
        .ok_or_else(|| MuxError::Control(format!("unrecognised tmux -V output: {text}")))?;
    let (next, ver) = if let Some(dev) = rest.strip_prefix("next-") {
        (true, dev)
    } else {
        (false, rest)
    };
    let (numeric, suffix) = split_suffix(ver);
    let (major_s, minor_s) = numeric
        .split_once('.')
        .ok_or_else(|| MuxError::Control(format!("unrecognised tmux version: {text}")))?;
    let major = major_s
        .parse::<u32>()
        .map_err(|_| MuxError::Control(format!("unrecognised tmux version: {text}")))?;
    let minor = minor_s
        .parse::<u32>()
        .map_err(|_| MuxError::Control(format!("unrecognised tmux version: {text}")))?;
    Ok(Version {
        major,
        minor,
        suffix: suffix.to_owned(),
        next,
    })
}

fn split_suffix(ver: &str) -> (&str, &str) {
    let end = ver
        .find(|ch: char| !ch.is_ascii_digit() && ch != '.')
        .unwrap_or(ver.len());
    match (ver.get(..end), ver.get(end..)) {
        (Some(numeric), Some(suffix)) => (numeric, suffix),
        _ => (ver, ""),
    }
}

/// Require `found` to be at least [`Version::minimum`].
pub(crate) fn ensure_supported(found: &Version) -> Result<(), MuxError> {
    if found.is_supported() {
        Ok(())
    } else {
        Err(MuxError::TmuxTooOld {
            found: found.clone(),
            min: Version::minimum(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_supported, parse_version};

    #[test]
    fn parses_supported_releases() {
        for raw in [
            "tmux 3.3\n",
            "tmux 3.4",
            "tmux 3.5a",
            "tmux next-3.6",
            "tmux 3.6",
        ] {
            let version = parse_version(raw).expect("parse");
            assert!(version.is_supported(), "{raw}");
            ensure_supported(&version).expect("gate");
        }
        assert_eq!(parse_version("tmux 3.5a").expect("parse").suffix, "a");
        assert!(parse_version("tmux next-3.6").expect("parse").next);
    }

    #[test]
    fn rejects_older_than_3_3() {
        let version = parse_version("tmux 3.2a").expect("parse");
        assert!(!version.is_supported());
        let err = ensure_supported(&version).expect_err("too old");
        let message = err.to_string();
        assert!(message.contains("3.2a"), "{message}");
        assert!(message.contains("3.3"), "{message}");
    }
}
