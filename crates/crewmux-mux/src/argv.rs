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

/// External doorbell. tmux expands `#{hook_pane}` inside the single quotes.
///
/// The configured binary is embedded so the hook does not depend on `PATH`.
/// `rename-window` is the 3.3a signal: that release has no `%paste-buffer-changed`,
/// and the `%subscription-changed` backstop only fires once a second.
#[must_use]
pub(crate) fn pane_died_hook(binary: &str, socket: &str, bell: &str) -> String {
    let script = format!(
        "{binary} -L {socket} -f /dev/null set-buffer -b rig-dead-#{{hook_pane}} x; {binary} -L {socket} -f /dev/null rename-window -t {bell} rig-dead-#{{hook_pane}}"
    );
    format!("run-shell -b {}", quote_control(&script))
}

#[must_use]
pub(crate) fn pane_died_hook_args(
    session: &str,
    binary: &str,
    socket: &str,
    bell: &str,
) -> Vec<String> {
    vec![
        "set-hook".to_owned(),
        "-t".to_owned(),
        exact_session_colon(session),
        "pane-died".to_owned(),
        pane_died_hook(binary, socket, bell),
    ]
}

/// `new-session`, and when `remain_on_exit` is set, the pane option and death
/// hook in the same tmux command. A later command can lose the race against
/// a process that exits immediately.
pub(crate) fn armed_session_args(
    spec: &SessionSpec,
    binary: &str,
    socket: &str,
    bell: &str,
) -> Result<Vec<String>, MuxError> {
    if spec.argv.iter().any(|arg| arg == ";") {
        return Err(MuxError::Forbidden(
            "refusing a ';' command argument; it would split the tmux command queue",
        ));
    }
    let mut args = new_session_args(spec);
    if spec.remain_on_exit {
        ensure_hook_token(binary)?;
        ensure_hook_token(socket)?;
        ensure_hook_token(bell)?;
        let session = canonical_session_name(&spec.name);
        args.push(";".to_owned());
        args.extend(remain_on_exit_args(&session, true));
        args.push(";".to_owned());
        args.extend(pane_died_hook_args(&session, binary, socket, bell));
    }
    Ok(args)
}

fn ensure_hook_token(text: &str) -> Result<(), MuxError> {
    let safe = !text.is_empty()
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-' | b'+' | b'@')
        });
    if safe {
        Ok(())
    } else {
        Err(MuxError::Control(format!(
            "tmux hook token is not shell-safe: {text}"
        )))
    }
}

/// Hidden window in the control session. Renames are the 3.3a death doorbell.
#[must_use]
pub(crate) fn bell_window_args() -> Vec<String> {
    vec![
        "new-window".to_owned(),
        "-d".to_owned(),
        "-t".to_owned(),
        format!("{CONTROL_SESSION}:"),
        "-n".to_owned(),
        "crewmux-bell".to_owned(),
        "-P".to_owned(),
        "-F".to_owned(),
        "#{window_id}".to_owned(),
        "sleep".to_owned(),
        "86400".to_owned(),
        ";".to_owned(),
        "set-option".to_owned(),
        "-w".to_owned(),
        "-t".to_owned(),
        format!("{CONTROL_SESSION}:crewmux-bell"),
        "automatic-rename".to_owned(),
        "off".to_owned(),
    ]
}

pub(crate) fn parse_bell_id(stdout: &str) -> Result<String, MuxError> {
    let id = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with('@'))
        .unwrap_or("");
    let digits = id.strip_prefix('@').unwrap_or("");
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(id.to_owned())
    } else {
        Err(MuxError::Control(format!(
            "bell window id missing: {stdout}"
        )))
    }
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
        "-N".to_owned(),
        "-t".to_owned(),
        target.to_owned(),
    ];
    match range {
        CaptureRange::Visible => {}
        CaptureRange::Joined => args.push("-J".to_owned()),
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
        Key::CtrlU => Some("C-u"),
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
            Key::Enter | Key::Escape | Key::CtrlC | Key::CtrlU | Key::Up | Key::Down | Key::Tab => {
                if let Some(name) = key_name(key) {
                    named.push(name);
                }
            }
        }
    }
    flush_named(&mut named, &mut batches);
    Ok(batches)
}

/// `set-option -g window-size` crashes tmux 3.5a while a control client is attached.
pub(crate) fn reject_global_window_size(scope: &OptScope, key: &str) -> Result<(), MuxError> {
    if key == "window-size" && matches!(scope, OptScope::Global) {
        Err(MuxError::Forbidden(
            "refusing a global window-size option; set it on the session",
        ))
    } else {
        Ok(())
    }
}

#[must_use]
pub(crate) fn set_option_args(scope: &OptScope, key: &str, value: &str) -> Vec<String> {
    match scope {
        OptScope::Global => vec![
            "set-option".to_owned(),
            "-g".to_owned(),
            key.to_owned(),
            value.to_owned(),
        ],
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
    use std::path::PathBuf;

    use super::{
        armed_session_args, backstop_command, canonical_session_name, capture_args, exact_session,
        invocation, paste_buffer_args, quote_control, send_keys_args, validate_keys,
    };
    use crate::error::MuxError;
    use crate::types::{CaptureRange, PasteOpts, SessionSpec};
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

    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().copied().map(str::to_owned).collect()
    }

    #[test]
    fn paste_flags_follow_opts_and_skip_enter() {
        let cases = [
            (
                PasteOpts {
                    bracketed: true,
                    preserve_lf: true,
                },
                owned(&["paste-buffer", "-t", "%1", "-b", "rig-01", "-d", "-p", "-r"]),
            ),
            (
                PasteOpts {
                    bracketed: false,
                    preserve_lf: true,
                },
                owned(&["paste-buffer", "-t", "%1", "-b", "rig-01", "-d", "-r"]),
            ),
            (
                PasteOpts {
                    bracketed: true,
                    preserve_lf: false,
                },
                owned(&["paste-buffer", "-t", "%1", "-b", "rig-01", "-d", "-p"]),
            ),
            (
                PasteOpts {
                    bracketed: false,
                    preserve_lf: false,
                },
                owned(&["paste-buffer", "-t", "%1", "-b", "rig-01", "-d"]),
            ),
        ];
        for (opts, expected) in cases {
            let got = paste_buffer_args("%1", "rig-01", opts);
            assert_eq!(got, expected);
            assert!(!got.iter().any(|arg| arg == "Enter"));
        }
        assert!(PasteOpts::default().bracketed);
        assert!(PasteOpts::default().preserve_lf);
    }

    #[test]
    fn remain_on_exit_shares_the_new_session_command() {
        let mut spec = SessionSpec::new("demo", PathBuf::from("/tmp"));
        spec.remain_on_exit = true;
        spec.argv = vec!["true".to_owned()];
        let args = armed_session_args(&spec, "/usr/bin/tmux", "sock", "@9").expect("arm");
        let semis = args.iter().filter(|arg| arg.as_str() == ";").count();
        assert_eq!(semis, 2);
        assert!(args.iter().any(|arg| arg == "new-session"));
        assert!(args.iter().any(|arg| arg == "remain-on-exit"));
        let hook = args
            .iter()
            .find(|arg| arg.contains("run-shell"))
            .expect("hook");
        assert!(hook.contains("/usr/bin/tmux"), "{hook}");
        assert!(hook.contains("-f /dev/null"), "{hook}");
        assert!(hook.contains("-L sock"), "{hook}");
        assert!(hook.contains("rename-window"), "{hook}");
        assert!(hook.contains("@9"), "{hook}");
        assert!(hook.contains("rig-dead-"), "{hook}");
        spec.argv = vec![";".to_owned()];
        let err = armed_session_args(&spec, "tmux", "sock", "@1").expect_err("semi");
        assert!(matches!(err, MuxError::Forbidden(_)), "{err}");
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

    #[test]
    fn joined_capture_rejoins_wrapped_rows() {
        let args = capture_args("%1", CaptureRange::Joined);
        assert!(args.iter().any(|arg| arg == "-J"), "{args:?}");
        assert!(!args.iter().any(|arg| arg == "-S"), "{args:?}");
        let visible = capture_args("%1", CaptureRange::Visible);
        assert!(!visible.iter().any(|arg| arg == "-J"));
    }

    #[test]
    fn ctrl_u_is_sent_as_c_u() {
        let args = send_keys_args("%1", &[Key::CtrlU]).expect("keys");
        let batch = args.first().expect("batch");
        assert!(batch.iter().any(|arg| arg == "C-u"), "{batch:?}");
        assert!(
            !batch.iter().any(|arg| arg == "Enter"),
            "clear is C-u alone: {batch:?}"
        );
    }
}
