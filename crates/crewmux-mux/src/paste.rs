// Derived from openrig packages/daemon/src/adapters/tmux.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Paste sequence from spike S1a: load a 0600 temp file, `paste-buffer -d -p -r`, no Enter.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use crate::argv::{delete_buffer_args, load_buffer_args, paste_buffer_args};
use crate::error::MuxError;
use crate::exec::TmuxExec;
use crate::types::{PasteOpts, PasteReceipt};

/// macOS discards unread pty input at 1024 bytes (`MAX_INPUT` / `TTYHOG`).
/// tmux can flush one `paste-buffer` as a single write, so each write stays
/// under that even when the payload is one long line.
const PASTE_CHUNK: usize = 512;

/// Seconds between chunk writes. Long enough for the pane to drain 512 bytes,
/// short enough that a 64 KiB paste still finishes inside the tmux exec timeout.
const CHUNK_GAP: &str = "0.002";

/// tmux 3.3a rejects a client command whose packed argv (`strlen + 1` per
/// argument) exceeds `MAX_IMSGSIZE` (16384) minus the command header. Each
/// batch stays under this so a long temp path still fits.
const CLIENT_BUDGET: usize = 14_000;

/// tmux 3.7+ `cmd_unpack_argv` rejects `argc > 1000` and reports it as
/// `command too long`, even when the packed bytes still fit.
const CLIENT_ARGS: usize = 1000;

const BRACKET_START: &str = "\u{1b}[200~";
const BRACKET_END: &str = "\u{1b}[201~";

/// `load-buffer` from a mode-0600 file, then `paste-buffer`. Deletes the buffer on failure
/// and always removes the temp file.
///
/// A payload longer than [`PASTE_CHUNK`] is still one paste: one bracket pair
/// around every chunk, and a tmux delay between the writes. Separate
/// `paste-buffer -p` calls would each close the bracket and the pane would see
/// a cut-off paste. The chunks may take several tmux clients so the argv stays
/// under the 3.3a command-length limit; the bracket stays open across them.
pub(crate) async fn perform_paste(
    exec: &dyn TmuxExec,
    target: &str,
    text: &str,
    opts: PasteOpts,
) -> Result<PasteReceipt, MuxError> {
    let parts = paste_chunks(text);
    let buffer = if parts.len() <= 1 {
        paste_chunk(exec, target, text, opts).await?
    } else {
        paste_parts(exec, target, &parts, opts).await?
    };
    Ok(PasteReceipt {
        buffer,
        bytes: text.len(),
        bracketed: opts.bracketed,
    })
}

/// Optional bracket open, each chunk, a delay, optional close.
///
/// The delay is `run-shell -d` with no shell command, so the server waits and
/// then continues the same command queue. Batches that would exceed
/// [`CLIENT_BUDGET`] or [`CLIENT_ARGS`] go out as later clients; the bracket
/// is opened on the first and closed on the last. A failed queue can leave
/// the pane inside the open bracket; the close is sent again on that path.
async fn paste_parts(
    exec: &dyn TmuxExec,
    target: &str,
    parts: &[&str],
    opts: PasteOpts,
) -> Result<String, MuxError> {
    let mut staged = Vec::with_capacity(parts.len());
    for part in parts {
        let buffer = fresh_buffer();
        let path = std::env::temp_dir().join(format!("rig-paste-{buffer}.txt"));
        if let Err(err) = write_secret(&path, part.as_bytes()).await {
            remove_paths(&staged).await;
            return Err(err);
        }
        staged.push((buffer, path));
    }
    let batches = command_batches(target, &staged, opts);
    let mut ran = Ok(());
    for batch in &batches {
        if let Err(err) = exec.run(batch).await {
            ran = Err(err);
            break;
        }
    }
    remove_paths(&staged).await;
    let Some((last, _)) = staged.last() else {
        return Err(MuxError::Control("paste produced no buffer".to_owned()));
    };
    match ran {
        Ok(()) => Ok(last.clone()),
        Err(err) => {
            close_failed_paste(exec, target, &staged, opts.bracketed).await;
            Err(err)
        }
    }
}

async fn close_failed_paste(
    exec: &dyn TmuxExec,
    target: &str,
    staged: &[(String, PathBuf)],
    bracketed: bool,
) {
    if bracketed {
        drop(exec.run(&send_literal(target, BRACKET_END)).await);
    }
    for (buffer, _) in staged {
        drop(exec.run(&delete_buffer_args(buffer)).await);
    }
}

fn command_batches(
    target: &str,
    staged: &[(String, PathBuf)],
    opts: PasteOpts,
) -> Vec<Vec<String>> {
    let mut bare = opts;
    bare.bracketed = false;
    let mut batches = Vec::new();
    let mut args = Vec::new();
    if opts.bracketed {
        push_cmd(&mut args, send_literal(target, BRACKET_START));
    }
    for (index, (buffer, path)) in staged.iter().enumerate() {
        let gap = index > 0 && !args.is_empty();
        let piece = chunk_piece(target, buffer, path, bare, gap);
        if batch_full(&args, &piece) {
            batches.push(std::mem::take(&mut args));
            push_cmd(&mut args, chunk_piece(target, buffer, path, bare, false));
        } else {
            push_cmd(&mut args, piece);
        }
    }
    if opts.bracketed {
        let close = send_literal(target, BRACKET_END);
        if batch_full(&args, &close) {
            batches.push(std::mem::take(&mut args));
        }
        push_cmd(&mut args, close);
    }
    if !args.is_empty() {
        batches.push(args);
    }
    batches
}

fn chunk_piece(target: &str, buffer: &str, path: &Path, opts: PasteOpts, gap: bool) -> Vec<String> {
    let mut piece = Vec::new();
    if gap {
        push_cmd(&mut piece, gap_cmd());
    }
    push_cmd(&mut piece, load_buffer_args(buffer, path));
    push_cmd(&mut piece, paste_buffer_args(target, buffer, opts));
    piece
}

fn batch_full(existing: &[String], piece: &[String]) -> bool {
    if existing.is_empty() {
        return false;
    }
    joined_span(existing, piece) > CLIENT_BUDGET || joined_args(existing, piece) > CLIENT_ARGS
}

fn joined_args(existing: &[String], piece: &[String]) -> usize {
    existing.len().saturating_add(1).saturating_add(piece.len())
}

/// Packed size of `existing`, a joining `;`, and `piece`, matching tmux's
/// `strlen(arg) + 1` check.
fn joined_span(existing: &[String], piece: &[String]) -> usize {
    let join = if existing.is_empty() {
        0
    } else {
        ";".len().saturating_add(1)
    };
    argv_span(existing)
        .saturating_add(join)
        .saturating_add(argv_span(piece))
}

fn argv_span(args: &[String]) -> usize {
    args.iter().map(|arg| arg.len().saturating_add(1)).sum()
}

fn push_cmd(args: &mut Vec<String>, cmd: Vec<String>) {
    if !args.is_empty() {
        args.push(";".to_owned());
    }
    args.extend(cmd);
}

fn send_literal(target: &str, text: &str) -> Vec<String> {
    vec![
        "send-keys".to_owned(),
        "-l".to_owned(),
        "-t".to_owned(),
        target.to_owned(),
        text.to_owned(),
    ]
}

fn gap_cmd() -> Vec<String> {
    vec![
        "run-shell".to_owned(),
        "-d".to_owned(),
        CHUNK_GAP.to_owned(),
    ]
}

async fn remove_paths(staged: &[(String, PathBuf)]) {
    for (_, path) in staged {
        drop(tokio::fs::remove_file(path).await);
    }
}

fn paste_chunks(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return vec![text];
    }
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = chunk_end(rest);
        let Some((head, tail)) = rest.split_at_checked(end) else {
            break;
        };
        if head.is_empty() {
            break;
        }
        out.push(head);
        rest = tail;
    }
    if out.is_empty() { vec![text] } else { out }
}

fn chunk_end(text: &str) -> usize {
    let mut end = text.len().min(PASTE_CHUNK);
    while end > 0 && !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    end
}

fn fresh_buffer() -> String {
    let generated = ulid::Generator::new()
        .generate()
        .unwrap_or_else(ulid::Overflow::commit_overflow_increment);
    format!("rig-{generated}")
}

async fn paste_chunk(
    exec: &dyn TmuxExec,
    target: &str,
    text: &str,
    opts: PasteOpts,
) -> Result<String, MuxError> {
    let buffer = fresh_buffer();
    let path = std::env::temp_dir().join(format!("rig-paste-{buffer}.txt"));
    write_secret(&path, text.as_bytes()).await?;
    let loaded = exec.run(&load_buffer_args(&buffer, &path)).await;
    let result = match loaded {
        Ok(_) => match exec.run(&paste_buffer_args(target, &buffer, opts)).await {
            Ok(_) => Ok(buffer),
            Err(err) => {
                drop(exec.run(&delete_buffer_args(&buffer)).await);
                Err(err)
            }
        },
        Err(err) => Err(err),
    };
    drop(tokio::fs::remove_file(&path).await);
    result
}

async fn write_secret(path: &PathBuf, bytes: &[u8]) -> Result<(), MuxError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true).mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    drop(file);
    let permissions = std::fs::Permissions::from_mode(0o600);
    tokio::fs::set_permissions(path, permissions).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use super::perform_paste;
    use crate::error::MuxError;
    use crate::exec::{ExecOutput, TmuxExec};
    use crate::types::PasteOpts;

    struct Rec {
        calls: Mutex<Vec<Vec<String>>>,
        mode: Mutex<Option<u32>>,
        fail_paste: bool,
        files: Mutex<Vec<Vec<u8>>>,
    }

    #[async_trait]
    impl TmuxExec for Rec {
        async fn run(&self, args: &[String]) -> Result<ExecOutput, MuxError> {
            for cmd in commands(args) {
                if cmd.first().map(String::as_str) != Some("load-buffer") {
                    continue;
                }
                let path = cmd.last().expect("path");
                let meta = tokio::fs::metadata(path).await.expect("temp file");
                *self.mode.lock().await = Some(meta.permissions().mode() & 0o777);
                let bytes = tokio::fs::read(path).await.unwrap_or_default();
                self.files.lock().await.push(bytes);
            }
            self.calls.lock().await.push(args.to_vec());
            if self.fail_paste
                && commands(args)
                    .iter()
                    .any(|cmd| cmd.first().map(String::as_str) == Some("paste-buffer"))
            {
                return Err(MuxError::Exec {
                    argv: args.to_vec(),
                    code: Some(1),
                    stderr: "can't find pane".to_owned(),
                });
            }
            Ok(ExecOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn commands(args: &[String]) -> Vec<&[String]> {
        let mut out = Vec::new();
        let mut start = 0usize;
        for (index, arg) in args.iter().enumerate() {
            if arg == ";" {
                out.push(&args[start..index]);
                start = index.saturating_add(1);
            }
        }
        if start < args.len() {
            out.push(&args[start..]);
        }
        out
    }

    #[tokio::test]
    async fn paste_is_load_then_paste_without_enter_and_mode_0600() {
        let exec = Rec {
            calls: Mutex::new(Vec::new()),
            mode: Mutex::new(None),
            fail_paste: false,
            files: Mutex::new(Vec::new()),
        };
        let receipt = perform_paste(&exec, "%1", "hello\n", PasteOpts::default())
            .await
            .expect("paste");
        assert!(receipt.buffer.starts_with("rig-"));
        assert!(receipt.bracketed);
        assert_eq!(receipt.bytes, "hello\n".len());
        let calls = exec.calls.lock().await.clone();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].first().map(String::as_str), Some("load-buffer"));
        assert_eq!(
            calls[1],
            vec![
                "paste-buffer",
                "-t",
                "%1",
                "-b",
                receipt.buffer.as_str(),
                "-d",
                "-p",
                "-r",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
        assert!(!calls.iter().flatten().any(|arg| arg == "Enter"));
        assert_eq!(*exec.mode.lock().await, Some(0o600));
        let path = calls[0].last().expect("path");
        assert!(
            tokio::fs::metadata(path).await.is_err(),
            "temp file removed"
        );
    }

    #[tokio::test]
    async fn long_paste_is_split_under_the_macos_tty_limit() {
        let exec = Rec {
            calls: Mutex::new(Vec::new()),
            mode: Mutex::new(None),
            fail_paste: false,
            files: Mutex::new(Vec::new()),
        };
        let mut text = "W".repeat(1000);
        text.insert(511, '\u{21a9}');
        let receipt = perform_paste(&exec, "%1", &text, PasteOpts::default())
            .await
            .expect("paste");
        assert_eq!(receipt.bytes, text.len());
        let calls = exec.calls.lock().await.clone();
        assert_eq!(calls.len(), 1, "two chunks fit in one tmux client");
        let argv = calls.first().expect("argv");
        assert!(super::argv_span(argv) <= super::CLIENT_BUDGET);
        assert!(argv.len() <= super::CLIENT_ARGS);
        let cmds = commands(calls.first().expect("argv"));
        let pastes: Vec<&&[String]> = cmds
            .iter()
            .filter(|cmd| cmd.first().map(String::as_str) == Some("paste-buffer"))
            .collect();
        assert!(pastes.len() > 1, "{}", pastes.len());
        assert!(
            pastes.iter().all(|cmd| !cmd.iter().any(|arg| arg == "-p")),
            "chunks must not close the bracket"
        );
        assert!(pastes.iter().all(|cmd| cmd.iter().any(|arg| arg == "-r")));
        let gaps = cmds
            .iter()
            .filter(|cmd| cmd.first().map(String::as_str) == Some("run-shell"))
            .count();
        assert_eq!(gaps, pastes.len() - 1);
        let keys: Vec<&&[String]> = cmds
            .iter()
            .filter(|cmd| cmd.first().map(String::as_str) == Some("send-keys"))
            .collect();
        assert_eq!(
            keys.first().and_then(|cmd| cmd.last()).map(String::as_str),
            Some("\u{1b}[200~")
        );
        assert_eq!(
            keys.get(1).and_then(|cmd| cmd.last()).map(String::as_str),
            Some("\u{1b}[201~")
        );
        assert_eq!(keys.len(), 2);
        let files = exec.files.lock().await.clone();
        assert_eq!(files.len(), pastes.len());
        let mut joined = String::new();
        for bytes in &files {
            assert!(bytes.len() <= 512, "{}", bytes.len());
            joined.push_str(&String::from_utf8_lossy(bytes));
        }
        assert_eq!(joined, text);
        assert!(!calls.iter().flatten().any(|arg| arg == "Enter"));
    }

    #[tokio::test]
    async fn long_unbracketed_paste_has_no_markers() {
        let exec = Rec {
            calls: Mutex::new(Vec::new()),
            mode: Mutex::new(None),
            fail_paste: false,
            files: Mutex::new(Vec::new()),
        };
        let text = "W".repeat(1000);
        perform_paste(
            &exec,
            "%1",
            &text,
            PasteOpts {
                bracketed: false,
                preserve_lf: true,
            },
        )
        .await
        .expect("paste");
        let calls = exec.calls.lock().await.clone();
        let cmds = commands(calls.first().expect("argv"));
        assert!(
            cmds.iter()
                .all(|cmd| cmd.first().map(String::as_str) != Some("send-keys"))
        );
        assert!(cmds.iter().all(|cmd| !cmd.iter().any(|arg| arg == "-p")));
    }

    #[test]
    fn long_temp_paths_split_before_the_imsg_limit() {
        let staged: Vec<(String, std::path::PathBuf)> = (0..128)
            .map(|index| {
                (
                    format!("rig-{index:026}"),
                    std::path::PathBuf::from(format!(
                        "/var/folders/ab/cdefghijklmnopqrstuv/T/rig-paste-rig-{index:026}.txt"
                    )),
                )
            })
            .collect();
        let batches = super::command_batches("%1", &staged, PasteOpts::default());
        assert!(batches.len() > 1, "{}", batches.len());
        let mut pastes = 0usize;
        let mut keys = Vec::new();
        for batch in &batches {
            assert!(
                super::argv_span(batch) <= super::CLIENT_BUDGET,
                "{}",
                super::argv_span(batch)
            );
            assert!(batch.len() <= super::CLIENT_ARGS, "{}", batch.len());
            for cmd in commands(batch) {
                match cmd.first().map(String::as_str) {
                    Some("paste-buffer") => {
                        pastes = pastes.saturating_add(1);
                        assert!(!cmd.iter().any(|arg| arg == "-p"));
                        assert!(cmd.iter().any(|arg| arg == "-r"));
                    }
                    Some("send-keys") => keys.push(cmd.last().cloned()),
                    _ => {}
                }
            }
        }
        assert_eq!(pastes, staged.len());
        assert_eq!(
            keys.as_slice(),
            [
                Some("\u{1b}[200~".to_owned()),
                Some("\u{1b}[201~".to_owned())
            ]
        );
    }

    #[tokio::test]
    async fn wide_paste_is_one_bracket_across_clients() {
        let exec = Rec {
            calls: Mutex::new(Vec::new()),
            mode: Mutex::new(None),
            fail_paste: false,
            files: Mutex::new(Vec::new()),
        };
        let text = "x".repeat(64 * 1024);
        perform_paste(&exec, "%1", &text, PasteOpts::default())
            .await
            .expect("paste");
        let calls = exec.calls.lock().await.clone();
        assert!(calls.len() > 1, "64 KiB exceeds one imsg");
        let mut pastes = 0usize;
        let mut keys = Vec::new();
        for call in &calls {
            assert!(super::argv_span(call) <= super::CLIENT_BUDGET);
            assert!(call.len() <= super::CLIENT_ARGS, "{}", call.len());
            for cmd in commands(call) {
                match cmd.first().map(String::as_str) {
                    Some("paste-buffer") => {
                        pastes = pastes.saturating_add(1);
                        assert!(!cmd.iter().any(|arg| arg == "-p"));
                    }
                    Some("send-keys") => keys.push(cmd.last().cloned()),
                    _ => {}
                }
            }
        }
        assert_eq!(
            keys.as_slice(),
            [
                Some("\u{1b}[200~".to_owned()),
                Some("\u{1b}[201~".to_owned())
            ]
        );
        let files = exec.files.lock().await.clone();
        assert_eq!(files.len(), pastes);
        let mut joined = String::new();
        for bytes in &files {
            assert!(bytes.len() <= 512, "{}", bytes.len());
            joined.push_str(&String::from_utf8_lossy(bytes));
        }
        assert_eq!(joined, text);
    }

    #[tokio::test]
    async fn failed_long_paste_closes_the_bracket() {
        let exec = Arc::new(Rec {
            calls: Mutex::new(Vec::new()),
            mode: Mutex::new(None),
            fail_paste: true,
            files: Mutex::new(Vec::new()),
        });
        let err = perform_paste(
            exec.as_ref(),
            "%missing",
            &"W".repeat(1000),
            PasteOpts::default(),
        )
        .await
        .expect_err("fail");
        assert!(matches!(err, MuxError::Exec { .. }));
        let calls = exec.calls.lock().await.clone();
        let close = calls.get(1).expect("bracket close");
        assert_eq!(close.last().map(String::as_str), Some("\u{1b}[201~"));
        let deletes = calls
            .iter()
            .filter(|cmd| cmd.first().map(String::as_str) == Some("delete-buffer"))
            .count();
        let files = exec.files.lock().await.len();
        assert_eq!(deletes, files);
        assert!(files > 1);
    }

    #[tokio::test]
    async fn failed_paste_deletes_the_buffer() {
        let exec = Arc::new(Rec {
            calls: Mutex::new(Vec::new()),
            mode: Mutex::new(None),
            fail_paste: true,
            files: Mutex::new(Vec::new()),
        });
        let err = perform_paste(exec.as_ref(), "%missing", "x", PasteOpts::default())
            .await
            .expect_err("fail");
        assert!(matches!(err, MuxError::Exec { .. }));
        let calls = exec.calls.lock().await.clone();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2].first().map(String::as_str), Some("delete-buffer"));
        assert_eq!(
            calls[2].get(2).map(String::as_str),
            calls[0].get(2).map(String::as_str)
        );
    }
}
