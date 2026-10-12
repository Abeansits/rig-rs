//! Match a capture to one row of a dialog table.

use crate::dialog::menu::{MenuOption, parse_menu};
use crate::dialog::table::{DialogParams, DialogSpec, TitleMatch};

/// A table row whose title and full option list are on screen.
pub struct Recognised {
    /// The row that matched.
    pub spec: &'static DialogSpec,
    /// Extracted folder, server, or versions.
    pub params: DialogParams,
    /// Choices in screen order.
    pub menu: Vec<MenuOption>,
}

/// Title matches and the parsed labels equal `spec.options`, in that order.
///
/// Anything less is unrecognised. A CLI that adds one option fails closed.
#[must_use]
pub fn recognise(table: &'static [DialogSpec], screen: &str) -> Option<Recognised> {
    let menu = parse_menu(screen);
    let labels: Vec<&str> = menu.iter().map(|option| option.label.as_str()).collect();
    for spec in table {
        if !title_matches(&spec.title, screen) {
            continue;
        }
        if labels.len() != spec.options.len() {
            continue;
        }
        let same = labels
            .iter()
            .zip(spec.options.iter())
            .all(|(found, want)| found == want);
        if same {
            return Some(Recognised {
                spec,
                params: (spec.params)(screen),
                menu,
            });
        }
    }
    None
}

fn title_matches(title: &TitleMatch, text: &str) -> bool {
    match title {
        TitleMatch::Contains(needle) => text.lines().any(|line| line.contains(needle)),
        TitleMatch::AllOf(needles) => needles
            .iter()
            .all(|needle| text.lines().any(|line| line.contains(needle))),
    }
}
