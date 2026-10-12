// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Pure projection. A policy error never becomes a successful empty census.

use serde::Serialize;

use crate::coverage::{Coverage, CoverageRow};
use crate::detector::DetectorId;
use crate::error::Error;
use crate::evaluate::evaluate;
use crate::observation::HealthObservation;
use crate::policy::HealthPolicy;
use crate::record::{HEALTH_LIST_SCHEMA, HealthRecord};
use crate::time::iso_timestamp;

/// Bounds for one projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthQuery {
    /// Page size, 1..=200. Ceremony findings are reserved ahead of this cap.
    pub limit: u32,
    /// Clock the records are stamped with when a source has no earlier time.
    pub now: crewmux_core::Timestamp,
    /// When set, only this severity is returned.
    pub severity: Option<crate::record::Severity>,
    /// When set, only this status is returned. When absent, cleared rows are hidden.
    pub status: Option<crate::record::Status>,
}

/// What one source returned. `Err` discards that source's observations.
#[derive(Clone, Debug)]
pub struct SourceRead {
    /// Source name, used when the read fails.
    pub name: String,
    /// `Ok` observations plus coverage. `None` coverage means the input was not evaluated.
    pub result: Result<(Vec<HealthObservation>, Option<Coverage>), String>,
}

/// A source the query can skip without calling [`HealthSource::read`].
pub trait HealthSource {
    /// Stable name for coverage and failures.
    fn name(&self) -> &str;
    /// Detectors this source can supply.
    fn detectors(&self) -> &[DetectorId];
    /// Read observations. `Ok((_, None))` is coverage `none`, not a healthy census.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the read fails. The projection names that failure
    /// and drops the source's observations.
    fn read(
        &self,
        query: &HealthQuery,
    ) -> Result<(Vec<HealthObservation>, Option<Coverage>), Error>;
}

/// The list document.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthProjection {
    /// Schema token.
    pub schema: &'static str,
    /// Newest evaluation time, or `None` when nothing was projected.
    pub evaluated_at: Option<String>,
    /// Findings before the page cap.
    pub total: u32,
    /// Requested page size.
    pub limit: u32,
    /// `total` exceeded `limit`.
    pub truncated: bool,
    /// Page. Ceremony amplification is ordered ahead of context samples.
    pub records: Vec<HealthRecord>,
    /// Per-source coverage, including `none` and `unavailable`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageRow>,
    /// Named source failures. Also present in [`Self::coverage`]. Not a second census.
    #[serde(skip)]
    pub failures: Vec<CoverageRow>,
}

/// Project `reads` under an already-loaded policy.
///
/// # Errors
///
/// Returns [`Error::Limit`] when `query.limit` is outside 1..=200.
pub fn project(
    policy: &HealthPolicy,
    reads: &[SourceRead],
    query: &HealthQuery,
) -> Result<HealthProjection, Error> {
    checked_limit(query.limit)?;
    let mut records = Vec::new();
    let mut coverage = Vec::new();
    let mut failures = Vec::new();
    let stamped = iso_timestamp(query.now);
    for read in reads {
        fold_read(
            policy,
            read,
            &stamped,
            &mut records,
            &mut coverage,
            &mut failures,
        );
    }
    records.retain(|record| policy.allows(record.detector) && visible(query, record));
    reserve_ceremony(&mut records);
    let total = u32::try_from(records.len()).unwrap_or(u32::MAX);
    let keep = usize::try_from(query.limit).unwrap_or(usize::MAX);
    let truncated = records.len() > keep;
    if truncated {
        records.truncate(keep);
    }
    let evaluated_at = (!records.is_empty()).then(|| iso_timestamp(query.now));
    Ok(HealthProjection {
        schema: HEALTH_LIST_SCHEMA,
        evaluated_at,
        total,
        limit: query.limit,
        truncated,
        records,
        coverage,
        failures,
    })
}

/// Load `policy`, skip disabled sources, then [`project`].
///
/// A `Err` policy fails the query before any source is read.
///
/// # Errors
///
/// Returns the policy error, or [`Error::Limit`].
pub fn run_query(
    policy: Result<HealthPolicy, Error>,
    sources: &[&dyn HealthSource],
    query: &HealthQuery,
) -> Result<HealthProjection, Error> {
    let policy = policy?;
    checked_limit(query.limit)?;
    let mut reads = Vec::new();
    for source in sources {
        if skipped(&policy, source.detectors()) {
            continue;
        }
        let result = match source.read(query) {
            Ok(value) => Ok(value),
            Err(err) => Err(err.to_string()),
        };
        reads.push(SourceRead {
            name: source.name().to_owned(),
            result,
        });
    }
    project(&policy, &reads, query)
}

/// `limit` must be an integer from 1 to 200.
///
/// # Errors
///
/// Returns [`Error::Limit`] otherwise.
pub fn checked_limit(limit: u32) -> Result<u32, Error> {
    if (1..=200).contains(&limit) {
        Ok(limit)
    } else {
        Err(Error::Limit)
    }
}

fn visible(query: &HealthQuery, record: &HealthRecord) -> bool {
    let severity_ok = query
        .severity
        .is_none_or(|severity| record.severity == severity);
    let status_ok = match query.status {
        Some(status) => record.status == status,
        None => record.status != crate::record::Status::Cleared,
    };
    severity_ok && status_ok
}

fn skipped(policy: &HealthPolicy, detectors: &[DetectorId]) -> bool {
    !detectors.is_empty() && detectors.iter().all(|id| !policy.allows(*id))
}

fn fold_read(
    policy: &HealthPolicy,
    read: &SourceRead,
    stamped: &str,
    records: &mut Vec<HealthRecord>,
    coverage: &mut Vec<CoverageRow>,
    failures: &mut Vec<CoverageRow>,
) {
    match &read.result {
        Err(reason) => {
            let row = stamp(CoverageRow::failed(&read.name, reason.clone()), stamped);
            failures.push(row.clone());
            coverage.push(row);
        }
        Ok((_, None)) => coverage.push(stamp(
            CoverageRow::not_evaluated(
                &read.name,
                "input table is not in this schema; not evaluated",
            ),
            stamped,
        )),
        Ok((observations, Some(bounds))) => {
            coverage.push(stamp(CoverageRow::bounded(&read.name, *bounds), stamped));
            for observation in observations {
                for mut record in evaluate(observation, policy) {
                    record.coverage = Some(*bounds);
                    records.push(record);
                }
            }
        }
    }
}

fn stamp(mut row: CoverageRow, at: &str) -> CoverageRow {
    row.evaluated_at = Some(at.to_owned());
    row
}

fn reserve_ceremony(records: &mut [HealthRecord]) {
    records.sort_by(|left, right| {
        ceremony_rank(left)
            .cmp(&ceremony_rank(right))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn ceremony_rank(record: &HealthRecord) -> u8 {
    u8::from(record.detector != DetectorId::CeremonyAmplification)
}
