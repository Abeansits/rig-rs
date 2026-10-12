//! Per-seat pane transcripts under `transcripts/<rig>/<seat>`.
//!
//! The `.log` keeps terminal bytes. The `.txt` twin strips CSI and OSC.

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
                        if let Some((rig, seat)) = place_of(&app, &pane_id).await {
                            let text = String::from_utf8_lossy(bytes.as_ref()).into_owned();
                            let home = app.home.clone();
                            let cap = app.transcript_cap;
                            drop(
                                tokio::task::spawn_blocking(move || {
                                    append(&home, &rig, &seat, &text, cap)
                                })
                                .await,
                            );
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn place_of(app: &App, pane_id: &str) -> Option<(String, String)> {
    let pane = pane_id.to_owned();
    let seats = crate::db::read(&app.store, |conn| crewmux_core::SeatRepo::list_all(conn))
        .await
        .ok()?;
    let seat = seats
        .into_iter()
        .find(|seat| seat.pane_id.as_deref() == Some(pane.as_str()))?;
    let rigs = crate::db::read(&app.store, |conn| crewmux_core::RigRepo::list(conn))
        .await
        .ok()?;
    let rig = rigs.into_iter().find(|rig| rig.id == seat.rig_id)?;
    Some((rig.name.as_str().to_owned(), seat.name.to_string()))
}

fn seat_dir(home: &std::path::Path, rig: &str, seat: &str) -> std::path::PathBuf {
    home.join("transcripts").join(rig).join(seat)
}

fn append(
    home: &std::path::Path,
    rig: &str,
    seat: &str,
    ansi: &str,
    cap: u64,
) -> std::io::Result<()> {
    let dir = seat_dir(home, rig, seat);
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

/// Plain pane transcript bytes. A missing directory is empty.
pub(crate) fn plain_bytes(
    home: &std::path::Path,
    rig: &str,
    seat: &str,
) -> std::io::Result<Vec<u8>> {
    let dir = seat_dir(home, rig, seat);
    let mut files = generation_paths(&dir)?;
    files.sort_by_key(|(generation, _)| *generation);
    let mut out = Vec::new();
    for (_, path) in files {
        match std::fs::read(&path) {
            Ok(chunk) => out.extend(chunk),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(out)
}

pub(crate) fn plain_len(home: &std::path::Path, rig: &str, seat: &str) -> std::io::Result<u64> {
    let bytes = plain_bytes(home, rig, seat)?;
    Ok(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
}

pub(crate) async fn len_of(home: &std::path::Path, rig: &str, seat: &str) -> std::io::Result<u64> {
    let home = home.to_path_buf();
    let rig = rig.to_owned();
    let seat = seat.to_owned();
    match tokio::task::spawn_blocking(move || plain_len(&home, &rig, &seat)).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::other("transcript length task stopped")),
    }
}

/// Wait until `needle` appears at or after `before` in the plain transcript.
pub(crate) async fn wait_for_paste(
    home: &std::path::Path,
    rig: &str,
    seat: &str,
    before: u64,
    needle: &str,
) {
    let home = home.to_path_buf();
    let rig = rig.to_owned();
    let seat = seat.to_owned();
    let needle = needle.to_owned();
    for _ in 0..100 {
        let found = {
            let home = home.clone();
            let rig = rig.clone();
            let seat = seat.clone();
            let needle = needle.clone();
            tokio::task::spawn_blocking(move || landed(&home, &rig, &seat, before, &needle)).await
        };
        if found.ok().and_then(Result::ok).unwrap_or(false) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

fn landed(
    home: &std::path::Path,
    rig: &str,
    seat: &str,
    before: u64,
    needle: &str,
) -> std::io::Result<bool> {
    if needle.is_empty() {
        return Ok(true);
    }
    let bytes = plain_bytes(home, rig, seat)?;
    let start = usize::try_from(before).unwrap_or(0).min(bytes.len());
    let tail = bytes.get(start..).unwrap_or(&[]);
    Ok(String::from_utf8_lossy(tail).contains(needle))
}

fn generation_paths(dir: &std::path::Path) -> std::io::Result<Vec<(u64, std::path::PathBuf)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        let Some(stem) = text.strip_suffix(".txt") else {
            continue;
        };
        let Ok(generation) = stem.parse::<u64>() else {
            continue;
        };
        files.push((generation, entry.path()));
    }
    Ok(files)
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek() {
                Some('[') => skip_csi(&mut chars),
                Some(']') => skip_osc(&mut chars),
                _ => {}
            }
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

/// OSC: `ESC ] ... BEL` or `ESC ] ... ESC \`.
fn skip_osc(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    if chars.peek() != Some(&']') {
        return;
    }
    chars.next();
    while let Some(next) = chars.next() {
        if next == '\u{7}' {
            break;
        }
        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next();
            break;
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

    #[test]
    fn osc_title_is_removed_from_the_twin() {
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}body"), "body");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{1b}\\body"), "body");
    }
}
