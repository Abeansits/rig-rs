//! Operator threshold. Out-of-contract values disable compaction.

use serde_json::Value;

/// A compaction threshold percent, or disabled when the raw value is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Threshold(Option<u8>);

impl Threshold {
    /// A validated percent in `1..=100`.
    #[must_use]
    pub fn some(percent: u8) -> Self {
        if (1..=100).contains(&percent) {
            Self(Some(percent))
        } else {
            Self(None)
        }
    }

    /// Disabled policy.
    #[must_use]
    pub const fn disabled() -> Self {
        Self(None)
    }

    /// Parse a JSON number.
    ///
    /// `0`, `101`, a non-integer, `NaN`, and a non-number all disable the policy.
    /// JSON has no `NaN`; [`Self::parse_text`] accepts the token `NaN`.
    #[must_use]
    pub fn parse(raw: &Value) -> Self {
        let Some(number) = raw.as_f64() else {
            return Self(None);
        };
        Self::from_f64(number)
    }

    /// Parse a settings token such as `80`, `80.0`, `1.5`, or `NaN`.
    #[must_use]
    pub fn parse_text(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("nan") {
            return Self(None);
        }
        let Ok(number) = trimmed.parse::<f64>() else {
            return Self(None);
        };
        Self::from_f64(number)
    }

    /// The percent, when the value is in contract.
    #[must_use]
    pub const fn get(self) -> Option<u8> {
        self.0
    }

    fn from_f64(number: f64) -> Self {
        if !number.is_finite() {
            return Self(None);
        }
        Self(whole_percent(number))
    }
}

fn whole_percent(number: f64) -> Option<u8> {
    let mut percent = 1u8;
    loop {
        let exact = f64::from(percent);
        if (number - exact).abs() < 1e-9 {
            return Some(percent);
        }
        if percent == 100 {
            return None;
        }
        percent = percent.saturating_add(1);
    }
}
