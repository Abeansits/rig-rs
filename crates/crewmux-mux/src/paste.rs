// Derived from openrig packages/daemon/src/adapters/tmux.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Paste sequence from spike S1a: load a 0600 temp file, `paste-buffer -d -p -r`, no Enter.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use tokio::io::AsyncWriteExt;

use crate::argv::{delete_buffer_args, load_buffer_args, paste_buffer_args};
use crate::error::MuxError;
use crate::exec::TmuxExec;
use crate::types::{PasteOpts, PasteReceipt};

/// `load-buffer` from a mode-0600 file, then `paste-buffer`. Deletes the buffer on failure
/// and always removes the temp file.
pub(crate) async fn perform_paste(
    exec: &dyn TmuxExec,
    target: &str,
    text: &str,
    opts: PasteOpts,
) -> Result<PasteReceipt, MuxError> {
    let generated = ulid::Generator::new()
        .generate()
        .unwrap_or_else(ulid::Overflow::commit_overflow_increment);
    let buffer = format!("rig-{generated}");
    let path = std::env::temp_dir().join(format!("rig-paste-{buffer}.txt"));
    write_secret(&path, text.as_bytes()).await?;
    let loaded = exec.run(&load_buffer_args(&buffer, &path)).await;
    let result = match loaded {
        Ok(_) => match exec.run(&paste_buffer_args(target, &buffer, opts)).await {
            Ok(_) => Ok(PasteReceipt {
                buffer: buffer.clone(),
                bytes: text.len(),
                bracketed: opts.bracketed,
            }),
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
    }

    #[async_trait]
    impl TmuxExec for Rec {
        async fn run(&self, args: &[String]) -> Result<ExecOutput, MuxError> {
            if args.first().map(String::as_str) == Some("load-buffer") {
                let path = args.last().expect("path");
                let meta = tokio::fs::metadata(path).await.expect("temp file");
                *self.mode.lock().await = Some(meta.permissions().mode() & 0o777);
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
        assert_eq!(calls[1].first().map(String::as_str), Some("paste-buffer"));
        assert!(
            calls[1]
                .windows(2)
                .any(|pair| pair == ["-d", "-p"] || pair == ["-p", "-r"])
        );
        assert!(calls[1].iter().any(|arg| arg == "-r"));
        assert!(!calls.iter().flatten().any(|arg| arg == "Enter"));
        assert_eq!(*exec.mode.lock().await, Some(0o600));
        let path = calls[0].last().expect("path");
        assert!(
            tokio::fs::metadata(path).await.is_err(),
            "temp file removed"
        );
    }

    #[tokio::test]
    async fn failed_paste_deletes_the_buffer() {
        let exec = Arc::new(Rec {
            calls: Mutex::new(Vec::new()),
            mode: Mutex::new(None),
            fail_paste: true,
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
