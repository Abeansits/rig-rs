// Derived from openrig packages/tui/src/state.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Pure update. This function does not read the clock, the environment, or the terminal.

use crewmux_api::{SeatId, SeatState, StreamEvent};

use crate::action::Action;
use crate::msg::{
    AttachOutcome, AttachTarget, Cmd, ControlCmd, HydrateFail, KeyEvent, KeyKind, MouseEvent, Msg,
};
use crate::rows::{self, find_seat};
use crate::state::{ConnState, FLASH_WINDOW, IDLE_TICK, RowFlash, RowKey, Section, State};

/// Apply one message. Replaying the same messages from the same state yields the same state.
#[must_use]
pub fn update(mut state: State, msg: Msg) -> (State, Vec<Cmd>) {
    let mut cmds = Vec::new();
    match msg {
        Msg::Key(key) => on_key(&mut state, &key, &mut cmds),
        Msg::Mouse(mouse) => on_mouse(&mut state, mouse, &mut cmds),
        Msg::Resize(cols, rows) => on_resize(&mut state, cols, rows),
        Msg::Hydrated(result) => on_hydrated(&mut state, result),
        Msg::Live(event) => on_live(&mut state, event),
        Msg::Conn(conn) => on_conn(&mut state, conn),
        Msg::Tick => on_tick(&mut state),
        Msg::AttachReturned(outcome) => on_returned(&mut state, outcome, &mut cmds),
        Msg::Control(cmd) => on_control(&mut state, cmd, &mut cmds),
        Msg::Painted(hits) => state.hits = hits,
    }
    recompute(&mut state);
    (state, cmds)
}

fn on_key(state: &mut State, key: &KeyEvent, cmds: &mut Vec<Cmd>) {
    match key.kind {
        KeyKind::Char('q') => cmds.push(Cmd::Quit),
        KeyKind::Char('r') => request_hydrate(state, cmds),
        KeyKind::Char('a') => attach_selection(state, cmds),
        KeyKind::Char('j') | KeyKind::Down => move_selection(state, 1),
        KeyKind::Char('k') | KeyKind::Up => move_selection(state, -1),
        KeyKind::Char(' ') => toggle_selection(state),
        KeyKind::Char(ch @ '1'..='5') => open_digit(state, ch),
        KeyKind::Enter => activate(state),
        KeyKind::Char(_) => {}
    }
}

fn open_digit(state: &mut State, ch: char) {
    state.section = match ch {
        '1' => Section::Explorer,
        '2' => Section::Detail,
        '3' => Section::Attention,
        '4' => Section::Health,
        '5' => Section::Activity,
        _ => state.section,
    };
    state.scroll = 0;
}

fn activate(state: &mut State) {
    match state.selection.clone() {
        Some(RowKey::Seat(_)) => {
            state.section = Section::Detail;
            state.scroll = 0;
        }
        Some(key) => toggle(state, &key),
        None => {}
    }
}

fn toggle_selection(state: &mut State) {
    if let Some(key) = state.selection.clone() {
        toggle(state, &key);
    }
}

fn on_mouse(state: &mut State, mouse: MouseEvent, cmds: &mut Vec<Cmd>) {
    if mouse.button & 64 != 0 {
        on_wheel(state, mouse);
        return;
    }
    if mouse.button != 0 {
        return;
    }
    let Some(action) = state.hits.hit(mouse.x, mouse.y).cloned() else {
        return;
    };
    apply_action(state, &action, cmds);
}

fn on_wheel(state: &mut State, mouse: MouseEvent) {
    let delta: i16 = if mouse.button & 1 != 0 { 3 } else { -3 };
    if mouse.x <= state.explorer_right {
        move_selection(state, delta);
    } else {
        scroll_by(state, delta);
    }
}

fn apply_action(state: &mut State, action: &Action, cmds: &mut Vec<Cmd>) {
    match action {
        Action::Select(key) => state.selection = Some(key.clone()),
        Action::ToggleDisclosure(key) => toggle(state, key),
        Action::OpenSection(section) => {
            state.section = *section;
            state.scroll = 0;
        }
        Action::Attach(seat) => attach_seat(state, seat, cmds),
        Action::Scroll(delta) => scroll_by(state, *delta),
        Action::Cross(_) => {
            state.section = Section::Detail;
            state.scroll = 0;
        }
    }
}

fn on_resize(state: &mut State, cols: u16, rows: u16) {
    state.size.cols = cols;
    state.size.rows = rows;
    state.explorer_right = crate::state::explorer_right(cols);
}

fn on_hydrated(state: &mut State, result: Result<crewmux_api::Snapshot, HydrateFail>) {
    state.load.in_flight = false;
    state.load.settled = true;
    match result {
        Ok(snapshot) => apply_snapshot(state, snapshot),
        Err(_) => state.load.stale = true,
    }
}

fn apply_snapshot(state: &mut State, snapshot: crewmux_api::Snapshot) {
    state.snapshot = snapshot;
    state.load.stale = false;
    state.load.connection = ConnState::Connected;
    state.load.last_success_at = Some(state.now);
    for key in rows::disclosure_keys(&state.snapshot) {
        if !state.collapsed.contains(&key) {
            state.expanded.insert(key);
        }
    }
    if !selection_alive(state) {
        state.selection = None;
    }
}

fn on_live(state: &mut State, event: StreamEvent) {
    match event {
        StreamEvent::SeatStateChanged { seat, state: next } => {
            set_seat_state(state, &seat, next);
            flash(state, RowKey::Seat(seat));
        }
        StreamEvent::SeatDied { seat } => {
            set_seat_state(state, &seat, SeatState::Dead);
            flash(state, RowKey::Seat(seat));
        }
        StreamEvent::Activity { seat, summary } => {
            state.snapshot.activity.insert(
                0,
                crewmux_api::ActivityItem {
                    seat: seat.clone(),
                    summary,
                },
            );
            state.snapshot.activity.truncate(20);
            flash(state, RowKey::Seat(seat));
        }
    }
}

fn set_seat_state(state: &mut State, id: &SeatId, next: SeatState) {
    for seat in &mut state.snapshot.seats {
        if &seat.id == id {
            seat.state = next;
            return;
        }
    }
}

fn flash(state: &mut State, key: RowKey) {
    state.flashes.retain(|row| row.key != key);
    state.flashes.push(RowFlash { key, at: state.now });
}

fn on_conn(state: &mut State, conn: ConnState) {
    state.load.connection = conn;
    if conn != ConnState::Connected {
        state.load.stale = true;
    }
}

fn on_tick(state: &mut State) {
    state.now = state.now.checked_add(IDLE_TICK).unwrap_or(state.now);
    if state.load.connection == ConnState::Dropped {
        state.load.connection = ConnState::Reconnecting;
    }
    state
        .flashes
        .retain(|row| row.at.checked_add(FLASH_WINDOW).unwrap_or(row.at) > state.now);
}

fn on_returned(state: &mut State, outcome: AttachOutcome, cmds: &mut Vec<Cmd>) {
    match outcome {
        AttachOutcome::Detached => {}
        AttachOutcome::DetachedWithNotice(notice) => state.notice = Some(notice),
        AttachOutcome::Failed(message) => state.notice = Some(message),
        AttachOutcome::SessionGone => state.notice = Some("session is gone".to_owned()),
    }
    request_hydrate(state, cmds);
}

fn on_control(state: &mut State, cmd: ControlCmd, cmds: &mut Vec<Cmd>) {
    match cmd {
        ControlCmd::Select { seat, reply } => {
            select_seat(state, &seat);
            reply_ok(reply, cmds);
        }
        ControlCmd::Section { section, reply } => {
            state.section = section;
            state.scroll = 0;
            reply_ok(reply, cmds);
        }
        ControlCmd::Refresh { reply } => {
            request_hydrate(state, cmds);
            reply_ok(reply, cmds);
        }
        ControlCmd::Attach { seat, reply } => {
            select_seat(state, &seat);
            attach_seat(state, &seat, cmds);
            reply_ok(reply, cmds);
        }
        ControlCmd::Unknown { verb, reply } => {
            let error = format!("unknown command \"{verb}\"");
            cmds.push(Cmd::Reply(
                reply,
                serde_json::json!({"ok": false, "error": error}),
            ));
        }
    }
}

fn reply_ok(reply: crate::msg::ControlReplyId, cmds: &mut Vec<Cmd>) {
    cmds.push(Cmd::Reply(reply, serde_json::json!({"ok": true})));
}

fn request_hydrate(state: &mut State, cmds: &mut Vec<Cmd>) {
    state.load.in_flight = true;
    cmds.push(Cmd::Hydrate);
}

fn attach_selection(state: &mut State, cmds: &mut Vec<Cmd>) {
    let Some(RowKey::Seat(id)) = state.selection.clone() else {
        state.notice = Some("select a seat to attach".to_owned());
        return;
    };
    attach_seat(state, &id, cmds);
}

fn attach_seat(state: &mut State, id: &SeatId, cmds: &mut Vec<Cmd>) {
    let Some(seat) = find_seat(&state.snapshot, id) else {
        state.notice = Some("seat is gone".to_owned());
        return;
    };
    if !seat.state.can_attach() {
        state.notice = Some(format!("cannot attach a {} seat", seat.state.label()));
        return;
    }
    let Some(session) = seat.session.clone() else {
        state.notice = Some("seat has no session".to_owned());
        return;
    };
    match plan(state, id, &session) {
        Ok(target) => cmds.push(Cmd::Attach(target)),
        Err(notice) => state.notice = Some(notice),
    }
}

fn plan(
    state: &State,
    id: &SeatId,
    session: &crewmux_api::SessionId,
) -> Result<AttachTarget, String> {
    let socket = state.env.socket_name.clone();
    match state.env.inside_tmux.as_deref() {
        None => Ok(AttachTarget::Tmux {
            socket,
            session: session.clone(),
        }),
        Some(name) if name == state.env.socket_name => Ok(AttachTarget::SwitchClient {
            socket,
            session: session.clone(),
        }),
        Some(_) => match state.env.launcher {
            Some(kind) => Ok(AttachTarget::Launcher {
                kind,
                seat: id.clone(),
                socket,
                session: session.clone(),
            }),
            None => Err(
                "already inside another tmux server; configure a launcher or detach before attaching"
                    .to_owned(),
            ),
        },
    }
}

fn select_seat(state: &mut State, id: &SeatId) {
    if find_seat(&state.snapshot, id).is_some() {
        state.selection = Some(RowKey::Seat(id.clone()));
    }
}

fn selection_alive(state: &State) -> bool {
    match &state.selection {
        Some(RowKey::Seat(id)) => find_seat(&state.snapshot, id).is_some(),
        Some(key) => rows::disclosure_keys(&state.snapshot)
            .iter()
            .any(|row| row == key),
        None => true,
    }
}

fn toggle(state: &mut State, key: &RowKey) {
    if state.expanded.contains(key) {
        state.expanded.remove(key);
        state.collapsed.insert(key.clone());
    } else {
        state.collapsed.remove(key);
        state.expanded.insert(key.clone());
    }
}

fn move_selection(state: &mut State, delta: i16) {
    let rows = rows::visible_rows(state);
    let Some(last) = rows.len().checked_sub(1) else {
        return;
    };
    let current = state
        .selection
        .as_ref()
        .and_then(|key| rows.iter().position(|row| &row.key == key))
        .unwrap_or(0);
    let last_i = i32::try_from(last).unwrap_or(i32::MAX);
    let current_i = i32::try_from(current).unwrap_or(0);
    let next = current_i.saturating_add(i32::from(delta)).clamp(0, last_i);
    let Some(index) = usize::try_from(next).ok() else {
        return;
    };
    if let Some(row) = rows.get(index) {
        state.selection = Some(row.key.clone());
    }
}

fn scroll_by(state: &mut State, delta: i16) {
    let max = i32::from(state.content_max_offset);
    let next = i32::from(state.scroll)
        .saturating_add(i32::from(delta))
        .clamp(0, max);
    state.scroll = u16::try_from(next).unwrap_or(0);
}

fn recompute(state: &mut State) {
    let pane = state.size.rows.saturating_sub(2);
    let lines = u16::try_from(rows::content_rows(state).len()).unwrap_or(u16::MAX);
    state.content_max_offset = lines.saturating_sub(pane);
    if state.scroll > state.content_max_offset {
        state.scroll = state.content_max_offset;
    }
}
