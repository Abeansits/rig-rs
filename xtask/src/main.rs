//! Repository automation. `cargo xtask lint-mux` enforces the tmux control-client rules.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("lint-mux") => match lint_mux_at(&workspace_root()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                report(&message);
                ExitCode::from(1)
            }
        },
        Some(other) => {
            report(&format!("unknown task {other}"));
            ExitCode::from(2)
        }
        None => {
            report("usage: cargo xtask lint-mux");
            ExitCode::from(2)
        }
    }
}

fn report(message: &str) {
    #[expect(clippy::print_stderr, reason = "xtask reports task failures on stderr")]
    {
        eprintln!("{message}");
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn lint_mux_at(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    walk(root, &mut files)?;
    let attached = concat!("session_", "attached");
    let shells = [
        concat!("Command::new(\"", "sh", "\")"),
        concat!("Command::new(\"", "bash", "\")"),
        concat!("Command::new(\"", "dash", "\")"),
    ];
    for path in files {
        let text =
            std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        if mentions_attached(&path, &text, attached) {
            return Err(format!(
                "{} contains `{attached}` outside the clients.rs doc comment",
                path.display()
            ));
        }
        if path_is_mux(&path) {
            for shell in shells {
                if text.contains(shell) {
                    return Err(format!("{} contains `{shell}`", path.display()));
                }
            }
        }
    }
    Ok(())
}

fn mentions_attached(path: &Path, text: &str, needle: &str) -> bool {
    for line in text.lines() {
        if !line.contains(needle) || doc_comment_in_clients(path, line) {
            continue;
        }
        return true;
    }
    false
}

fn doc_comment_in_clients(path: &Path, line: &str) -> bool {
    let display = path.to_string_lossy();
    if !display.ends_with("crates/crewmux-mux/src/clients.rs") {
        return false;
    }
    let trimmed = line.trim_start();
    trimmed.starts_with("//!") || trimmed.starts_with("///")
}

fn path_is_mux(path: &Path) -> bool {
    path.to_string_lossy().contains("crates/crewmux-mux/")
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::lint_mux_at;

    #[test]
    fn workspace_mux_sources_pass() {
        let root = super::workspace_root();
        lint_mux_at(&root).expect("lint");
    }

    #[test]
    fn a_code_mention_outside_the_doc_comment_fails() {
        let root = std::env::temp_dir().join(format!("xtask-lint-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&root));
        let clients = root.join("crates/crewmux-mux/src/clients.rs");
        std::fs::create_dir_all(clients.parent().expect("parent")).expect("dirs");
        let needle = concat!("session_", "attached");
        std::fs::write(&clients, format!("//! mention {needle} only here\n")).expect("write");
        lint_mux_at(&root).expect("docs are allowed");
        let bad = root.join("crates/crewmux-mux/src/other.rs");
        std::fs::write(&bad, format!("fn probe() {{ let _ = \"{needle}\"; }}\n")).expect("write");
        let err = lint_mux_at(&root).expect_err("code mention");
        assert!(err.contains("other.rs"), "{err}");
        drop(std::fs::remove_dir_all(&root));
    }

    #[test]
    fn shell_command_construction_in_mux_fails() {
        let root = std::env::temp_dir().join(format!("xtask-shell-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&root));
        let file = root.join("crates/crewmux-mux/src/exec.rs");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dirs");
        let call = concat!("Command::new(\"", "sh", "\")");
        std::fs::write(&file, format!("fn probe() {{ let _ = {call}; }}\n")).expect("write");
        let err = lint_mux_at(&root).expect_err("shell");
        assert!(err.contains("exec.rs"), "{err}");
        drop(std::fs::remove_dir_all(&root));
    }
}
