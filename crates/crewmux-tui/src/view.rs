// Derived from openrig packages/tui/src/render.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Paint the TUI. Hit targets are the widget rects passed to `render_widget`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;

use crate::action::{Action, HitMap, HitTarget};
use crate::rows::{self, ContentRow, VisibleRow};
use crate::state::{RowKey, State};

/// Draw `state` and return the hit map for this frame.
#[must_use]
pub fn view(state: &State, frame: &mut Frame) -> HitMap {
    let mut hits = HitMap::default();
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    let Some(header) = chunks.first().copied() else {
        return hits;
    };
    let Some(body) = chunks.get(1).copied() else {
        return hits;
    };
    let Some(footer) = chunks.get(2).copied() else {
        return hits;
    };
    paint_line(frame, header, &header_text(state), Style::default());
    paint_body(frame, body, state, &mut hits);
    paint_line(frame, footer, &footer_text(state), Style::default());
    hits
}

fn paint_body(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let width = state.explorer_right.min(area.width);
    let cols = Layout::horizontal([Constraint::Length(width), Constraint::Min(1)]).split(area);
    let Some(explorer) = cols.first().copied() else {
        return;
    };
    let Some(content) = cols.get(1).copied() else {
        return;
    };
    paint_explorer(frame, explorer, state, hits);
    paint_content(frame, content, state, hits);
}

fn paint_explorer(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let rows = rows::visible_rows(state);
    if rows.is_empty() {
        paint_line(frame, row_rect(area, 0), "no seats", Style::default());
        return;
    }
    for (index, row) in rows.iter().enumerate() {
        let Some(rect) = row_at(area, index) else {
            break;
        };
        paint_explorer_row(frame, rect, row, flashed(state, &row.key), hits);
    }
}

fn paint_explorer_row(
    frame: &mut Frame,
    area: Rect,
    row: &VisibleRow,
    flashed: bool,
    hits: &mut HitMap,
) {
    let style = if flashed {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let indent = u16::from(row.depth).saturating_mul(2);
    if row.disclosure {
        push_disclosure(frame, area, row, indent, style, hits);
    }
    let text_at = if row.disclosure {
        indent.saturating_add(2)
    } else {
        indent
    };
    let text_area = shifted(area, text_at);
    paint_line(frame, text_area, &row.title, style);
    if let Some(target) = target_of(area, Action::Select(row.key.clone())) {
        hits.targets.push(target);
    }
}

fn push_disclosure(
    frame: &mut Frame,
    area: Rect,
    row: &VisibleRow,
    indent: u16,
    style: Style,
    hits: &mut HitMap,
) {
    let cell = Rect {
        x: area.x.saturating_add(indent),
        y: area.y,
        width: 1,
        height: 1,
    };
    let glyph = if row.open { "v" } else { ">" };
    paint_line(frame, cell, glyph, style);
    if let Some(target) = target_of(cell, Action::ToggleDisclosure(row.key.clone())) {
        hits.targets.push(target);
    }
}

fn paint_content(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let rows = rows::content_rows(state);
    let skip = usize::from(state.scroll);
    for (index, row) in rows.iter().skip(skip).enumerate() {
        let Some(rect) = row_at(area, index) else {
            break;
        };
        paint_content_row(frame, rect, row, hits);
    }
}

fn paint_content_row(frame: &mut Frame, area: Rect, row: &ContentRow, hits: &mut HitMap) {
    paint_line(frame, area, &row.text, Style::default());
    let Some(action) = row.action.clone() else {
        return;
    };
    if let Some(target) = target_of(area, action) {
        hits.content.push(target);
    }
}

fn paint_line(frame: &mut Frame, area: Rect, text: &str, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(fit(text, area.width)).style(style), area);
}

fn header_text(state: &State) -> String {
    let mark = if state.load.shows_live() {
        " live"
    } else if state.load.stale {
        " stale"
    } else {
        ""
    };
    format!(
        "rig-rs  {}  {}{mark}",
        state.section.as_str(),
        state.load.connection_label()
    )
}

fn footer_text(state: &State) -> String {
    state
        .notice
        .clone()
        .unwrap_or_else(|| "j/k move  a attach  r refresh  q quit  1-5 section".to_owned())
}

fn flashed(state: &State, key: &RowKey) -> bool {
    state.flashes.iter().any(|flash| &flash.key == key)
}

fn row_rect(area: Rect, row: u16) -> Rect {
    Rect {
        x: area.x,
        y: area.y.saturating_add(row),
        width: area.width,
        height: 1,
    }
}

fn row_at(area: Rect, index: usize) -> Option<Rect> {
    let row = u16::try_from(index).ok()?;
    if row >= area.height {
        return None;
    }
    Some(row_rect(area, row))
}

fn shifted(area: Rect, columns: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(columns),
        y: area.y,
        width: area.width.saturating_sub(columns),
        height: area.height,
    }
}

fn target_of(area: Rect, action: Action) -> Option<HitTarget> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let x1 = area.x.saturating_add(1);
    let x2 = area.x.saturating_add(area.width);
    if x2 < x1 {
        return None;
    }
    Some(HitTarget {
        y: area.y.saturating_add(1),
        x1,
        x2,
        action,
    })
}

fn fit(text: &str, width: u16) -> String {
    text.chars().take(usize::from(width)).collect()
}
