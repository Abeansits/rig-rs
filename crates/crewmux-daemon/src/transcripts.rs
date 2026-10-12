//! Per-seat pane transcripts. ANSI is kept, and a stripped twin is written beside it.

use std::io::Write;

use crewmux_mux::MuxEvent;

use crate::state::App;

pub(crate) async fn run(app: App) {
    if !app.transcripts {
        return;
    }
    let mut events = app.mux.events();
    loop {
        tokio::select! {
            () = app.cancel.cancelled() => break,
            event = events.recv() => {
                match event {
                    Ok(MuxEvent::Output { pane_id, bytes }) => {
                        if let Some(seat) = seat_for(&app, &pane_id).await {
                            let text = String::from_utf8_lossy(bytes.as_ref()).into_owned();
                            let home = app.home.clone();
                            let cap = app.transcript_cap;
                            drop(tokio::task::spawn_blocking(move || append(&home, &seat, &text, cap)).await);
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn seat_for(app: &App, pane_id: &str) -> Option<String> {
    let pane = pane_id.to_owned();
    let seats = crate::db::read(&app.store, |conn| crewmux_core::SeatRepo::list_all(conn))
        .await
        .ok()?;
    seats.into_iter().find_map(|seat| {
        (seat.pane_id.as_deref() == Some(pane.as_str())).then(|| seat.name.to_string())
    })
}

fn append(home: &std::path::Path, seat: &str, ansi: &str, cap: u64) -> std::io::Result<()> {
    let dir = home.join("transcripts").join(seat);
    std::fs::create_dir_all(&dir)?;
    let generation = next_generation(&dir, cap)?;
    let log = dir.join(format!("{generation}.log"));
    let text = dir.join(format!("{generation}.txt"));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    let mut ansi_file = crewmux_api::open_private(&log, &mut options)?;
    ansi_file.write_all(ansi.as_bytes())?;
    let mut plain = crewmux_api::open_private(&text, &mut options)?;
    plain.write_all(strip_ansi(ansi).as_bytes())?;
    Ok(())
}

fn next_generation(dir: &std::path::Path, cap: u64) -> std::io::Result<u64> {
    let mut newest = 1u64;
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        let Some(stem) = text.strip_suffix(".log") else {
            continue;
        };
        if let Ok(generation) = stem.parse::<u64>() {
            newest = newest.max(generation);
        }
    }
    let path = dir.join(format!("{newest}.log"));
    let len = std::fs::metadata(&path).map_or(0, |meta| meta.len());
    if len >= cap {
        Ok(newest.saturating_add(1))
    } else {
        Ok(newest)
    }
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            skip_csi(&mut chars);
            continue;
        }
        out.push(ch);
    }
    out
}

fn skip_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    if chars.peek() == Some(&'[') {
        chars.next();
        for next in chars.by_ref() {
            if next.is_ascii_alphabetic() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn ansi_is_removed_from_the_twin() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
    }
}
