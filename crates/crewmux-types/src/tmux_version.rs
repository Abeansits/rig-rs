//! tmux version parsing and the 3.3 minimum.
//!
//! Slice 1b's `crewmux-mux` parser should call this module (or drop its copy)
//! when the branches merge. The shape matches that parser: `major`, `minor`,
//! a letter `suffix`, and `next` for `tmux next-X.Y`.

use std::fmt;

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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.next {
            write!(formatter, "next-")?;
        }
        write!(formatter, "{}.{}{}", self.major, self.minor, self.suffix)
    }
}

impl Version {
    /// Oldest tmux this project will use.
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
///
/// # Errors
///
/// Returns a message when the text is not a tmux version line.
pub fn parse_version(raw: &str) -> Result<Version, String> {
    let text = raw.trim();
    let Some(rest) = text.strip_prefix("tmux ") else {
        return Err(format!("unrecognised tmux -V output: {text}"));
    };
    let (next, ver) = if let Some(dev) = rest.strip_prefix("next-") {
        (true, dev)
    } else {
        (false, rest)
    };
    let (numeric, suffix) = split_suffix(ver);
    let Some((major_s, minor_s)) = numeric.split_once('.') else {
        return Err(format!("unrecognised tmux version: {text}"));
    };
    let Ok(major) = major_s.parse::<u32>() else {
        return Err(format!("unrecognised tmux version: {text}"));
    };
    let Ok(minor) = minor_s.parse::<u32>() else {
        return Err(format!("unrecognised tmux version: {text}"));
    };
    Ok(Version {
        major,
        minor,
        suffix: suffix.to_owned(),
        next,
    })
}

fn split_suffix(ver: &str) -> (&str, &str) {
    let end = ver
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_digit() && *ch != '.')
        .map_or(ver.len(), |(index, _)| index);
    match (ver.get(..end), ver.get(end..)) {
        (Some(numeric), Some(suffix)) => (numeric, suffix),
        _ => (ver, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::{Version, parse_version};

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
        }
        assert_eq!(parse_version("tmux 3.5a").expect("parse").suffix, "a");
        assert!(parse_version("tmux next-3.6").expect("parse").next);
        assert_eq!(
            parse_version("tmux 3.5a").expect("parse").to_string(),
            "3.5a"
        );
    }

    #[test]
    fn rejects_older_than_3_3() {
        let version = parse_version("tmux 3.2a").expect("parse");
        assert!(!version.is_supported());
        assert_eq!(version.major, 3);
        assert_eq!(version.minor, 2);
        assert!(!Version::minimum().next);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_version("not tmux").is_err());
        assert!(parse_version("tmux next").is_err());
    }

    proptest::proptest! {
        #[test]
        fn dotted_numbers_parse(major in 0u32..30, minor in 0u32..30) {
            let raw = format!("tmux {major}.{minor}");
            let version = parse_version(&raw).expect("parse");
            assert_eq!(version.major, major);
            assert_eq!(version.minor, minor);
            assert_eq!(version.suffix, "");
            assert_eq!(version.is_supported(), (major, minor) >= (3, 3));
        }
    }
}
