// Derived from openrig packages/daemon/src/adapters/tmux.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Paste sequence from spike S1a: load a 0600 temp file, `paste-buffer -d -p -r`, no Enter.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use tokio::io::AsyncWriteExt;

use crate::argv::{delete_buffer_args, load_buffer_args, paste_buffer_args};
use crate::error::MuxError;
use crate::exec::TmuxExec;
use crate::types::{PasteOpts, PasteReceipt};

/// macOS discards unread pty input at 1024 bytes (`MAX_INPUT` / `TTYHOG`).
/// tmux can flush one `paste-buffer` as a single write, so each command stays
/// under that even when the payload is one long line.
const PASTE_CHUNK: usize = 512;

/// `load-buffer` from a mode-0600 file, then `paste-buffer`. Deletes the buffer on failure
/// and always removes the temp file.
///
/// Payloads longer than [`PASTE_CHUNK`] are several pastes. A round trip between
/// them lets the pane drain the tty queue before the next write.
pub(crate) async fn perform_paste(
    exec: &dyn TmuxExec,
    target: &str,
    text: &str,
    opts: PasteOpts,
) -> Result<PasteReceipt, MuxError> {
    let parts = paste_chunks(text);
    let mut last = String::new();
    let mut pending = false;
    for chunk in &parts {
        if pending {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        last = paste_chunk(exec, target, chunk, opts).await?;
        pending = true;
    }
    Ok(PasteReceipt {
        buffer: last,
        bytes: text.len(),
        bracketed: opts.bracketed,
    })
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

async fn paste_chunk(
    exec: &dyn TmuxExec,
    target: &str,
    text: &str,
    opts: PasteOpts,
) -> Result<String, MuxError> {
    let generated = ulid::Generator::new()
        .generate()
        .unwrap_or_else(ulid::Overflow::commit_overflow_increment);
    let buffer = format!("rig-{generated}");
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
            if args.first().map(String::as_str) == Some("load-buffer") {
                let path = args.last().expect("path");
                let meta = tokio::fs::metadata(path).await.expect("temp file");
                *self.mode.lock().await = Some(meta.permissions().mode() & 0o777);
                let bytes = tokio::fs::read(path).await.unwrap_or_default();
                self.files.lock().await.push(bytes);
            }
            self.calls.lock().await.push(args.to_vec());
            if self.fail_paste && args.first().map(String::as_str) == Some("paste-buffer") {
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
        let pastes = calls
            .iter()
            .filter(|args| args.first().map(String::as_str) == Some("paste-buffer"))
            .count();
        let files = exec.files.lock().await.clone();
        assert!(pastes > 1, "{pastes}");
        assert_eq!(files.len(), pastes);
        let mut joined = String::new();
        for bytes in &files {
            assert!(bytes.len() <= 512, "{}", bytes.len());
            joined.push_str(&String::from_utf8_lossy(bytes));
        }
        assert_eq!(joined, text);
        assert!(!calls.iter().flatten().any(|arg| arg == "Enter"));
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
