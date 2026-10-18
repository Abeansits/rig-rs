//! One dialog driver. Each adapter supplies a recognised-dialog table.
//!
//! Enter is sent only from [`driver::DialogDriver::drive`], and only with a
//! [`driver::VerifiedCursor`] built in that module.

mod driver;
mod menu;
mod pace;
mod policy;
mod recognise;
#[cfg(test)]
mod settle_test;
mod table;

pub use driver::{DialogDriver, DriveOutcome, MuxPane, NavKey, PaneIo, VerifiedCursor};
pub use menu::{MenuOption, is_option_menu, parse_menu};
pub use pace::{DialogClock, DialogPace, ManualClock, WallClock};
pub use policy::{AnswerPolicy, EscalateReason, Plan, SeatContext, plan, same_dir};
pub use recognise::{Recognised, recognise};
pub use table::{Answer, DialogParams, DialogSpec, DialogTable, Gate, TitleMatch};
