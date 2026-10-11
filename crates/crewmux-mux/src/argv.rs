// Derived from openrig packages/daemon/src/adapters/tmux.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
// Derived from openrig packages/daemon/src/adapters/shell-quote.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Argv builders. Every tmux invocation is a `Vec` of arguments: no shell.
//!
//! Session names are canonicalised the way tmux stores them (`.` and `:` become
//! `_`). Targets use exact-match `=name`, so a prefix sibling is never selected.
//! A session whose name already starts with `=` is addressed as `==name`.

use std::path::Path;

use crewmux_types::Key;

use crate::error::MuxError;
use crate::types::{CONTROL_SESSION, CaptureRange, OptScope, PaneRef, PasteOpts, SessionSpec};

/// tmux rewrites `.` and `:` in session names to `_`.
#[must_use]
pub(crate) fn canonical_session_name(name: &str) -> String {
    name.chars()
        .map(|ch| if ch == '.' || ch == ':' { '_' } else { ch })
        .collect()
}

/// Exact session target (`=name`, or `==name` when the name itself starts with `=`).
#[must_use]
pub(crate) fn exact_session(name: &str) -> String {
    format!("={}", canonical_session_name(name))
}

/// Exact target for session options and hooks (`=name:`).
#[must_use]
pub(crate) fn exact_session_colon(name: &str) -> String {
    format!("{}:", exact_session(name))
}

/// `true` when `id` is a tmux pane id (`%` plus digits).
#[must_use]
pub(crate) fn is_pane_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix('%') else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
}

/// Exact pane target. A known `%N` id wins over the session name.
#[must_use]
pub(crate) fn pane_target(pane: &PaneRef) -> String {
    if let Some(id) = pane.pane_id.as_deref().filter(|id| is_pane_id(id)) {
        return id.to_owned();
    }
    let session = exact_session(&pane.session);
    match (pane.window, pane.pane) {
        (Some(window), Some(index)) => format!("{session}:{window}.{index}"),
        (Some(window), None) => format!("{session}:{window}"),
        (None, _) => format!("{session}:"),
    }
}

/// Single-quote a format string for the control-mode command channel.
///
/// `#` starts a comment there, so every `#{...}` is sent quoted.
#[must_use]
pub(crate) fn quote_control(text: &str) -> String {
    let mut out = String::from("'");
    for ch in text.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Full argv: `tmux -L <socket> -f /dev/null ...`.
#[must_use]
pub(crate) fn invocation(binary: &str, socket: &str, args: &[String]) -> Vec<String> {
    let mut full = vec![
        binary.to_owned(),
        "-L".to_owned(),
        socket.to_owned(),
        "-f".to_owned(),
        "/dev/null".to_owned(),
    ];
    full.extend(args.iter().cloned());
    full
}

/// `new-session` argv (without the binary or `-L`).
#[must_use]
pub(crate) fn new_session_args(spec: &SessionSpec) -> Vec<String> {
    let name = canonical_session_name(&spec.name);
    let mut args = vec![
        "new-session".to_owned(),
        "-d".to_owned(),
        "-s".to_owned(),
        name,
        "-c".to_owned(),
        spec.cwd.display().to_string(),
        "-x".to_owned(),
        spec.geometry.cols.to_string(),
        "-y".to_owned(),
        spec.geometry.rows.to_string(),
    ];
    for (key, value) in &spec.env {
        args.push("-e".to_owned());
        args.push(format!("{key}={value}"));
    }
    args.push("-P".to_owned());
    args.push("-F".to_owned());
    args.push("#{pane_id}|#{session_name}".to_owned());
    args.extend(spec.argv.iter().cloned());
    args
}

/// `window-size manual` so a later client cannot resize the window.
#[must_use]
pub(crate) fn manual_size_args(session: &str) -> Vec<String> {
    vec![
        "set-option".to_owned(),
        "-t".to_owned(),
        exact_session_colon(session),
        "window-size".to_owned(),
        "manual".to_owned(),
    ]
}

#[must_use]
pub(crate) fn resize_args(session: &str, cols: u16, rows: u16) -> Vec<String> {
    vec![
        "resize-window".to_owned(),
        "-t".to_owned(),
        exact_session_colon(session),
        "-x".to_owned(),
        cols.to_string(),
        "-y".to_owned(),
        rows.to_string(),
    ]
}

#[must_use]
pub(crate) fn remain_on_exit_args(session: &str, on: bool) -> Vec<String> {
    vec![
        "set-option".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        exact_session_colon(session),
        "remain-on-exit".to_owned(),
        if on {
            "on".to_owned()
        } else {
            "off".to_owned()
        },
    ]
}

/// External doorbell. tmux expands `#{hook_pane}` and runs the command itself.
#[must_use]
pub(crate) fn pane_died_hook(socket: &str) -> String {
    format!("run-shell -b 'tmux -L {socket} set-buffer -b rig-dead-#{{hook_pane}} x'")
}

#[must_use]
pub(crate) fn pane_died_hook_args(session: &str, socket: &str) -> Vec<String> {
    vec![
        "set-hook".to_owned(),
        "-t".to_owned(),
        exact_session_colon(session),
        "pane-died".to_owned(),
        pane_died_hook(socket),
    ]
}

#[must_use]
pub(crate) fn link_into_control_args(session: &str) -> Vec<String> {
    vec![
        "link-window".to_owned(),
        "-s".to_owned(),
        exact_session_colon(session),
        "-t".to_owned(),
        format!("{CONTROL_SESSION}:"),
    ]
}

#[must_use]
pub(crate) fn has_session_args(name: &str) -> Vec<String> {
    vec![
        "has-session".to_owned(),
        "-t".to_owned(),
        exact_session(name),
    ]
}

#[must_use]
pub(crate) fn kill_session_args(name: &str) -> Vec<String> {
    vec![
        "kill-session".to_owned(),
        "-t".to_owned(),
        exact_session(name),
    ]
}

/// Printable `|` separator. tmux 3.6 rewrites control characters in `-F` to `_`,
/// which breaks a tab-separated row. Session names may themselves contain `|`.
pub(crate) const PANE_FORMAT: &str = concat!(
    "#{pane_id}|#{window_id}|#{pane_pid}|#{pane_dead}|#{pane_dead_status}|",
    "#{pane_width}|#{pane_height}|#{pane_in_mode}|#{pane_current_command}|#{session_name}",
);

#[must_use]
pub(crate) fn list_panes_args() -> Vec<String> {
    vec![
        "list-panes".to_owned(),
        "-a".to_owned(),
        "-F".to_owned(),
        PANE_FORMAT.to_owned(),
    ]
}

pub(crate) const CLIENT_FORMAT: &str =
    "#{client_control_mode}|#{client_activity}|#{client_tty}|#{client_session}";

#[must_use]
pub(crate) fn list_clients_args() -> Vec<String> {
    vec![
        "list-clients".to_owned(),
        "-F".to_owned(),
        CLIENT_FORMAT.to_owned(),
    ]
}

#[must_use]
pub(crate) fn capture_args(target: &str, range: CaptureRange) -> Vec<String> {
    let mut args = vec![
        "capture-pane".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        target.to_owned(),
    ];
    match range {
        CaptureRange::Visible => {}
        CaptureRange::LastLines(lines) => {
            args.push("-S".to_owned());
            args.push(format!("-{lines}"));
        }
        CaptureRange::History(lines) => {
            args.push("-S".to_owned());
            args.push(format!("-{lines}"));
        }
    }
    args
}

#[must_use]
pub(crate) fn cursor_args(target: &str) -> Vec<String> {
    vec![
        "display".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        target.to_owned(),
        "#{cursor_x} #{cursor_y}".to_owned(),
    ]
}

#[must_use]
pub(crate) fn pane_mode_args(target: &str) -> Vec<String> {
    vec![
        "display".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        target.to_owned(),
        "#{pane_in_mode}".to_owned(),
    ]
}

#[must_use]
pub(crate) fn dead_status_args(pane_id: &str) -> Vec<String> {
    vec![
        "display".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        pane_id.to_owned(),
        "#{pane_dead} #{pane_dead_status}".to_owned(),
    ]
}

#[must_use]
pub(crate) fn load_buffer_args(buffer: &str, path: &Path) -> Vec<String> {
    vec![
        "load-buffer".to_owned(),
        "-b".to_owned(),
        buffer.to_owned(),
        path.display().to_string(),
    ]
}

/// `paste-buffer -t … -b … -d` plus optional `-p` and `-r`. Never Enter.
#[must_use]
pub(crate) fn paste_buffer_args(target: &str, buffer: &str, opts: PasteOpts) -> Vec<String> {
    let mut args = vec![
        "paste-buffer".to_owned(),
        "-t".to_owned(),
        target.to_owned(),
        "-b".to_owned(),
        buffer.to_owned(),
        "-d".to_owned(),
    ];
    if opts.bracketed {
        args.push("-p".to_owned());
    }
    if opts.preserve_lf {
        args.push("-r".to_owned());
    }
    args
}

#[must_use]
pub(crate) fn delete_buffer_args(buffer: &str) -> Vec<String> {
    vec![
        "delete-buffer".to_owned(),
        "-b".to_owned(),
        buffer.to_owned(),
    ]
}

/// Reject a literal that contains a newline.
///
/// Debug builds trip `debug_assert` as well as returning an error. Release
/// builds return the error only.
pub(crate) fn validate_keys(keys: &[Key]) -> Result<(), MuxError> {
    for key in keys {
        if let Key::Literal(text) = key {
            let newline = text.contains('\n');
            debug_assert!(!newline, "Key::Literal must be a single line; use paste");
            if newline {
                return Err(MuxError::Control(
                    "literal key contains a newline; use paste".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn key_name(key: &Key) -> Option<&str> {
    match key {
        Key::Enter => Some("Enter"),
        Key::Escape => Some("Escape"),
        Key::CtrlC => Some("C-c"),
        Key::Up => Some("Up"),
        Key::Down => Some("Down"),
        Key::Tab => Some("Tab"),
        Key::Literal(_) => None,
    }
}

/// One or more `send-keys` arg lists. Literals use `-l` and never include Enter implicitly.
pub(crate) fn send_keys_args(target: &str, keys: &[Key]) -> Result<Vec<Vec<String>>, MuxError> {
    validate_keys(keys)?;
    let mut batches = Vec::new();
    let mut named: Vec<&str> = Vec::new();
    let flush_named = |named: &mut Vec<&str>, batches: &mut Vec<Vec<String>>| {
        if named.is_empty() {
            return;
        }
        let mut args = vec!["send-keys".to_owned(), "-t".to_owned(), target.to_owned()];
        args.extend(named.drain(..).map(str::to_owned));
        batches.push(args);
    };
    for key in keys {
        match key {
            Key::Literal(text) => {
                flush_named(&mut named, &mut batches);
                batches.push(vec![
                    "send-keys".to_owned(),
                    "-l".to_owned(),
                    "-t".to_owned(),
                    target.to_owned(),
                    text.clone(),
                ]);
            }
            Key::Enter | Key::Escape | Key::CtrlC | Key::Up | Key::Down | Key::Tab => {
                if let Some(name) = key_name(key) {
                    named.push(name);
                }
            }
        }
    }
    flush_named(&mut named, &mut batches);
    Ok(batches)
}

#[must_use]
pub(crate) fn set_option_args(scope: &OptScope, key: &str, value: &str) -> Vec<String> {
    match scope {
        OptScope::Server => {
            vec![
                "set-option".to_owned(),
                "-s".to_owned(),
                key.to_owned(),
                value.to_owned(),
            ]
        }
        OptScope::Session(name) => vec![
            "set-option".to_owned(),
            "-t".to_owned(),
            exact_session_colon(name),
            key.to_owned(),
            value.to_owned(),
        ],
        OptScope::Window(target) => vec![
            "set-option".to_owned(),
            "-w".to_owned(),
            "-t".to_owned(),
            target.clone(),
            key.to_owned(),
            value.to_owned(),
        ],
        OptScope::Pane(pane) => vec![
            "set-option".to_owned(),
            "-p".to_owned(),
            "-t".to_owned(),
            pane_target(pane),
            key.to_owned(),
            value.to_owned(),
        ],
    }
}

/// Control-mode subscription on `#{pane_dead}` for every pane. Quoted because `#` comments.
#[must_use]
pub(crate) fn backstop_command() -> String {
    let spec = quote_control("crewmux-dead:%*:#{pane_dead}");
    format!("refresh-client -B {spec}")
}

#[cfg(test)]
mod tests {
    use super::{
        backstop_command, canonical_session_name, exact_session, invocation, paste_buffer_args,
        quote_control, send_keys_args, validate_keys,
    };
    use crate::types::PasteOpts;
    use crewmux_types::Key;

    #[test]
    fn socket_and_exact_names_round_trip() {
        let argv = invocation(
            "tmux",
            "rig-rs",
            &exact_session("worker")
                .split(' ')
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        assert_eq!(argv.get(1).map(String::as_str), Some("-L"));
        assert_eq!(argv.get(2).map(String::as_str), Some("rig-rs"));
        assert_eq!(canonical_session_name("foo.bar"), "foo_bar");
        assert_eq!(canonical_session_name("a:b"), "a_b");
        assert_eq!(exact_session("=worker"), "==worker");
        assert_eq!(exact_session("foo.bar"), "=foo_bar");
        assert_eq!(exact_session("worker-long"), "=worker-long");
        assert_ne!(exact_session("worker"), exact_session("worker-long"));
    }

    #[test]
    fn paste_flags_follow_opts_and_skip_enter() {
        let on = paste_buffer_args("%1", "rig-01", PasteOpts::default());
        assert_eq!(
            on,
            vec!["paste-buffer", "-t", "%1", "-b", "rig-01", "-d", "-p", "-r"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
        assert!(!on.iter().any(|arg| arg == "Enter"));
        let off = paste_buffer_args(
            "%1",
            "rig-01",
            PasteOpts {
                bracketed: false,
                preserve_lf: false,
            },
        );
        assert!(!off.iter().any(|arg| arg == "-p" || arg == "-r"));
        let keep_lf = paste_buffer_args(
            "%1",
            "rig-01",
            PasteOpts {
                bracketed: false,
                preserve_lf: true,
            },
        );
        assert!(keep_lf.iter().any(|arg| arg == "-r"));
        assert!(!keep_lf.iter().any(|arg| arg == "-p"));
    }

    #[test]
    fn control_formats_are_single_quoted() {
        let quoted = quote_control("#{pane_dead}");
        assert_eq!(quoted, "'#{pane_dead}'");
        let command = backstop_command();
        assert!(
            command.contains("'crewmux-dead:%*:#{pane_dead}'"),
            "{command}"
        );
        assert!(command.starts_with("refresh-client -B "));
    }

    #[test]
    fn literal_newline_is_rejected() {
        let keys = [Key::Literal("a\nb".to_owned())];
        let caught =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| validate_keys(&keys)));
        match caught {
            Err(_) => {}
            Ok(Err(err)) => assert!(err.to_string().contains("newline"), "{err}"),
            Ok(Ok(())) => panic!("newline literal was accepted"),
        }
        let ok = send_keys_args("%1", &[Key::Literal("hi".to_owned()), Key::Enter]).expect("keys");
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[0].get(1).map(String::as_str), Some("-l"));
        assert!(ok[1].iter().any(|arg| arg == "Enter"));
    }
}
