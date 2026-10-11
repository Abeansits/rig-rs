// Derived from openrig packages/tui/src/types.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Hit targets. A cell is a hit when `y == row && x1 <= x <= x2`.

use crewmux_api::SeatKey;

use crate::state::{RowKey, Section};

/// Where a content cross-link goes. v1 only names a spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossLink {
    /// Spec named on the seat.
    Spec {
        /// Spec name from the snapshot.
        name: String,
    },
}

/// What a hit cell does. Disclosure is registered before the row action so it wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move the explorer selection to this row.
    Select(RowKey),
    /// Open or close a rig or pod.
    ToggleDisclosure(RowKey),
    /// Switch the content pane.
    OpenSection(Section),
    /// Attach the seat with the external client.
    Attach(SeatKey),
    /// Scroll the content pane by this many lines. Negative scrolls up.
    Scroll(i16),
    /// Follow a cross-link in the seat detail.
    Cross(CrossLink),
}

/// One inclusive rectangle from a rendered widget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitTarget {
    /// 1-based row.
    pub y: u16,
    /// 1-based inclusive start column.
    pub x1: u16,
    /// 1-based inclusive end column.
    pub x2: u16,
    /// Action for this cell.
    pub action: Action,
}

/// Explorer targets plus content-pane targets. `hit` searches explorer first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HitMap {
    /// Explorer pane, disclosure cells before the row they sit on.
    pub targets: Vec<HitTarget>,
    /// Content pane.
    pub content: Vec<HitTarget>,
}

impl HitMap {
    /// First target that contains `(x, y)`. Disclosure wins because it is registered first.
    #[must_use]
    pub fn hit(&self, x: u16, y: u16) -> Option<&Action> {
        self.targets
            .iter()
            .chain(self.content.iter())
            .find(|target| contains(target, x, y))
            .map(|target| &target.action)
    }
}

fn contains(target: &HitTarget, x: u16, y: u16) -> bool {
    target.y == y && target.x1 <= x && x <= target.x2
}
