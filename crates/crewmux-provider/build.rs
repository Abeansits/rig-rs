//! Sets `codex_binary_present` when a `codex` executable is on `PATH`.
//! The AC8 test uses that cfg to ignore itself when the binary is absent.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(codex_binary_present)");
    println!("cargo:rerun-if-env-changed=PATH");
    if codex_on_path() {
        println!("cargo:rustc-cfg=codex_binary_present");
    }
}

fn codex_on_path() -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        if std::path::Path::new(dir).join("codex").is_file() {
            return true;
        }
    }
    false
}
