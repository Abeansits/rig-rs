//! Markdown counts for phase-exit quotes.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::types::{LedgerRow, LedgerStatus};

/// Counts per status and per slice.
#[must_use]
pub fn report(rows: &[LedgerRow]) -> String {
    let mut totals = Counts::default();
    let mut by_slice: BTreeMap<String, Counts> = BTreeMap::new();
    for row in rows {
        totals.add(row.status);
        let key = row
            .slice
            .clone()
            .unwrap_or_else(|| "(unassigned)".to_owned());
        by_slice.entry(key).or_default().add(row.status);
    }
    let mut out =
        String::from("# Parity ledger\n\n## By status\n\n| status | count |\n|---|---:|\n");
    for (label, count) in totals.pairs() {
        let _ = writeln!(out, "| {label} | {count} |");
    }
    let _ = writeln!(out, "| **total** | {} |", rows.len());
    out.push_str("\n## By slice\n\n| slice | todo | ported | reused | waived | diverged |\n|---|---:|---:|---:|---:|---:|\n");
    for (slice, counts) in &by_slice {
        let _ = writeln!(
            out,
            "| {slice} | {} | {} | {} | {} | {} |",
            counts.todo, counts.ported, counts.reused, counts.waived, counts.diverged
        );
    }
    out
}

#[derive(Default, Clone)]
struct Counts {
    todo: usize,
    ported: usize,
    reused: usize,
    waived: usize,
    diverged: usize,
}

impl Counts {
    fn add(&mut self, status: LedgerStatus) {
        match status {
            LedgerStatus::Todo => self.todo = self.todo.saturating_add(1),
            LedgerStatus::Ported => self.ported = self.ported.saturating_add(1),
            LedgerStatus::Reused => self.reused = self.reused.saturating_add(1),
            LedgerStatus::Waived => self.waived = self.waived.saturating_add(1),
            LedgerStatus::Diverged => self.diverged = self.diverged.saturating_add(1),
        }
    }

    fn pairs(&self) -> [(&'static str, usize); 5] {
        [
            ("todo", self.todo),
            ("ported", self.ported),
            ("reused", self.reused),
            ("waived", self.waived),
            ("diverged", self.diverged),
        ]
    }
}
