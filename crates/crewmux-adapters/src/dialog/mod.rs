//! Cursor-verified answers for known provider dialogs.
//!
//! Enter is sent only after a re-capture shows the cursor on the chosen line.
//! Login screens, the background-server menu, and unknown menus are not answered.

mod driver;
mod known;
mod options;
mod policy;

pub use driver::{DialogDriver, DriveResult, PaneIo};
pub use known::{DialogChoice, KnownDialog};
pub use options::{MenuOption, menu_options};
pub use policy::{AnswerPolicy, planned_choice};
