// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! What a source evaluated. Omitted items are not healthy.

use serde::{Deserialize, Serialize};

/// Counts from one bounded read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    /// Items the source looked at.
    pub evaluated: u32,
    /// Items left out of this read.
    pub omitted: u32,
    /// The source stopped because it hit its limit.
    #[serde(rename = "limit_hit")]
    pub limit_hit: bool,
}

/// How a source reported its read. `None` means the input was not evaluated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CoverageRow {
    /// Source name.
    pub source: String,
    /// `none` or `unavailable`. Absent when the read ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
    /// When the read was judged. Absent on a pure in-memory projection.
    #[serde(rename = "evaluatedAt", skip_serializing_if = "Option::is_none")]
    pub evaluated_at: Option<String>,
    /// Items evaluated. Absent when the read did not produce a count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluated: Option<u32>,
    /// Items not evaluated. Absent when the read did not produce a count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omitted: Option<u32>,
    /// The read stopped at a limit. Omitted when false so a failure is not a census.
    #[serde(skip_serializing_if = "omit_false")]
    pub limit_hit: bool,
    /// The census is incomplete.
    pub partial: bool,
    /// Why the read is partial or absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes a reference"
)]
fn omit_false(value: &bool) -> bool {
    !*value
}

impl CoverageRow {
    /// The input table is not in this schema. That is not a healthy census.
    #[must_use]
    pub fn not_evaluated(source: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            status: Some("none"),
            evaluated_at: None,
            evaluated: None,
            omitted: None,
            limit_hit: false,
            partial: true,
            reason: Some(reason.into()),
        }
    }

    /// A read that returned counts.
    #[must_use]
    pub fn bounded(source: impl Into<String>, coverage: Coverage) -> Self {
        let partial = coverage.limit_hit || coverage.omitted > 0;
        Self {
            source: source.into(),
            status: None,
            evaluated_at: None,
            evaluated: Some(coverage.evaluated),
            omitted: Some(coverage.omitted),
            limit_hit: coverage.limit_hit,
            partial,
            reason: partial.then(|| {
                "omitted items were not evaluated and are not a healthy verdict".to_owned()
            }),
        }
    }

    /// The read failed. Its observations are discarded. No count is invented.
    #[must_use]
    pub fn failed(source: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            status: Some("unavailable"),
            evaluated_at: None,
            evaluated: None,
            omitted: None,
            limit_hit: false,
            partial: true,
            reason: Some(reason.into()),
        }
    }
}
