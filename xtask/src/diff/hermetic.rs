// Derived from openrig packages/daemon/test/helpers/hermetic-env.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Scratch `HOME`, a private `tmux -L` socket, and a PATH shim that records argv.
//!
//! The TypeScript helper fail-closes on an inherited daemon target, injected clock,
//! or tmux attachment, and it forces a private socket. This port keeps those
//! refusals. The shim additionally appends JSONL and uses `-L` (the acceptance
//! spec) rather than `-S`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::HermeticHome;
use crate::error::{Error, io_err};

/// Inherited variables that retarget the CLI at a daemon this harness did not create.
const DAEMON_TARGET_ENV: [&str; 7] = [
    "OPENRIG_URL",
    "RIGGED_URL",
    "OPENRIG_HOST",
    "RIGGED_HOST",
    "OPENRIG_HOST_SELECTED",
    "OPENRIG_PORT",
    "RIGGED_PORT",
];

const CLOCK_ENV: [&str; 1] = ["OPENRIG_TEST_CLOCK_NOW"];
const TMUX_ENV: [&str; 1] = ["TMUX"];
const CREDENTIAL_ENV: [&str; 2] = ["OPENRIG_AUTH_BEARER_TOKEN", "RIGGED_AUTH_BEARER_TOKEN"];

static SOCKET_SEQ: AtomicU64 = AtomicU64::new(1);

const SHIM: &str = r#"#!/usr/bin/env python3
import json, os, sys
log = os.environ["TMUX_ARGV_LOG"]
real = os.environ["TMUX_REAL"]
socket = os.environ["TMUX_HARNESS_SOCKET"]
args = sys.argv[1:]
with open(log, "a", encoding="utf-8") as handle:
    handle.write(json.dumps(["tmux", *args]) + "\n")
cleaned = []
index = 0
while index < len(args):
    if args[index] in ("-L", "-S") and index + 1 < len(args):
        index += 2
        continue
    cleaned.append(args[index])
    index += 1
os.execv(real, [real, "-L", socket, *cleaned])
"#;

/// First non-empty inherited daemon-target variable, if any.
#[must_use]
pub fn foreign_daemon_target(vars: &[(String, String)]) -> Option<(String, String)> {
    first_set(vars, &DAEMON_TARGET_ENV)
}

/// First non-empty inherited clock variable, if any.
#[must_use]
pub fn ambient_clock(vars: &[(String, String)]) -> Option<(String, String)> {
    first_set(vars, &CLOCK_ENV)
}

/// First non-empty inherited tmux attachment, if any.
#[must_use]
pub fn ambient_tmux(vars: &[(String, String)]) -> Option<(String, String)> {
    first_set(vars, &TMUX_ENV)
}

impl HermeticHome {
    /// Scaffold from the current process environment.
    ///
    /// # Errors
    ///
    /// Refuses an inherited daemon target, clock, or tmux attachment before
    /// creating a directory. Returns [`Error::Io`] if the scratch dir cannot be made.
    pub fn create() -> Result<Self, Error> {
        let vars: Vec<(String, String)> = std::env::vars().collect();
        Self::from_vars(&vars)
    }

    /// Scaffold from an explicit environment. Tests pass a clean map.
    ///
    /// # Errors
    ///
    /// Same as [`HermeticHome::create`].
    pub fn from_vars(vars: &[(String, String)]) -> Result<Self, Error> {
        if let Some((name, value)) = foreign_daemon_target(vars) {
            return Err(Error::ForeignDaemon { name, value });
        }
        if let Some((name, value)) = ambient_clock(vars) {
            return Err(Error::AmbientClock { name, value });
        }
        if let Some((name, value)) = ambient_tmux(vars) {
            return Err(Error::AmbientTmux { name, value });
        }
        let root =
            tempfile::TempDir::new().map_err(|source| io_err("create scratch dir", source))?;
        let home = root.path().join("home");
        let rig_home = root.path().join("rig-home");
        let tmp = root.path().join("tmp");
        let tmux_tmp = root.path().join("tmux-tmp");
        let shim_dir = root.path().join("bin");
        for dir in [&home, &rig_home, &tmp, &tmux_tmp, &shim_dir] {
            fs::create_dir_all(dir).map_err(|source| io_err("create scratch subdir", source))?;
        }
        fs::create_dir_all(root.path().join("openrig-home"))
            .map_err(|source| io_err("create openrig home", source))?;
        let tmux_socket = fresh_socket_name();
        let tmux_log = root.path().join("tmux-argv.jsonl");
        fs::write(&tmux_log, b"").map_err(|source| io_err("create tmux argv log", source))?;
        let original_path = lookup(vars, "PATH").unwrap_or_default();
        let real_tmux = find_on_path(&original_path, "tmux");
        let shim = shim_dir.join("tmux");
        write_shim(&shim)?;
        let mut env = scrubbed_env(vars);
        env.insert("HOME".to_owned(), home.display().to_string());
        env.insert("TMPDIR".to_owned(), tmp.display().to_string());
        env.insert("RIG_HOME".to_owned(), rig_home.display().to_string());
        env.insert(
            "OPENRIG_HOME".to_owned(),
            root.path().join("openrig-home").display().to_string(),
        );
        env.insert(
            "XDG_CONFIG_HOME".to_owned(),
            home.join(".config").display().to_string(),
        );
        env.insert(
            "XDG_CACHE_HOME".to_owned(),
            home.join(".cache").display().to_string(),
        );
        env.insert(
            "XDG_DATA_HOME".to_owned(),
            home.join(".local/share").display().to_string(),
        );
        env.insert(
            "XDG_STATE_HOME".to_owned(),
            home.join(".local/state").display().to_string(),
        );
        env.insert("TMUX_TMPDIR".to_owned(), tmux_tmp.display().to_string());
        env.insert("TZ".to_owned(), "UTC".to_owned());
        env.insert(
            "PATH".to_owned(),
            format!("{}:{original_path}", shim_dir.display()),
        );
        env.insert("TMUX_ARGV_LOG".to_owned(), tmux_log.display().to_string());
        env.insert("TMUX_HARNESS_SOCKET".to_owned(), tmux_socket.clone());
        // Same name the shim forces with `-L`. The daemon's Drop kills and
        // unlinks `RIG_TMUX_SOCKET`. Leaving this unset makes that name the
        // shared `rig-rs`, so a shim-rewritten kill-server and the unlink hit
        // two different sockets and the developer's server disappears.
        env.insert("RIG_TMUX_SOCKET".to_owned(), tmux_socket.clone());
        if let Some(real) = &real_tmux {
            env.insert("TMUX_REAL".to_owned(), real.display().to_string());
        }
        Ok(Self {
            root,
            home,
            tmux_socket,
            path_prepend: vec![shim_dir],
            env,
            tmux_log,
            real_tmux,
            shim,
        })
    }

    /// Child environment. Does not include the parent `HOME` or `TMUX`.
    #[must_use]
    pub fn child_env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// `RIG_HOME` inside the scratch tree.
    #[must_use]
    pub fn rig_home(&self) -> PathBuf {
        self.root.path().join("rig-home")
    }

    /// True when `tmux` and `python3` were both found on the original `PATH`.
    #[must_use]
    pub fn can_probe_tmux(&self) -> bool {
        self.real_tmux.is_some() && self.env.contains_key("TMUX_REAL") && python3_on_path()
    }

    /// Run `tmux` through the shim (first on `PATH`) and return its exit code.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Message`] when tmux or python3 is missing, or [`Error::Io`]
    /// when the process cannot be spawned.
    pub fn invoke_tmux(&self, args: &[String]) -> Result<i32, Error> {
        if !self.can_probe_tmux() {
            return Err(Error::Message(
                "tmux shim was not invoked: tmux or python3 is not on PATH".to_owned(),
            ));
        }
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("exec tmux \"$@\"")
            .arg("sh")
            .args(args)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command
            .spawn()
            .map_err(|source| io_err("spawn tmux shim", source))?;
        let output = child
            .wait_with_output()
            .map_err(|source| io_err("wait for tmux shim", source))?;
        Ok(output.status.code().unwrap_or(1))
    }

    /// Argv lines the shim has recorded, in order.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] or [`Error::Json`] when the log cannot be read.
    pub fn tmux_argv(&self) -> Result<Vec<Vec<String>>, Error> {
        let text = fs::read_to_string(&self.tmux_log)
            .map_err(|source| io_err("read tmux argv log", source))?;
        let mut calls = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let parsed: Vec<String> = serde_json::from_str(line).map_err(|source| Error::Json {
                context: format!("tmux argv log line {index}"),
                source,
            })?;
            calls.push(parsed);
        }
        Ok(calls)
    }

    /// Path of the generated `tmux` wrapper.
    #[must_use]
    pub fn shim_path(&self) -> &Path {
        &self.shim
    }
}

impl Drop for HermeticHome {
    fn drop(&mut self) {
        if !harness_socket_owned(&self.tmux_socket) {
            return;
        }
        if let Some(real) = &self.real_tmux {
            let mut command = Command::new(real);
            command
                .args(["-L", &self.tmux_socket, "-f", "/dev/null", "kill-server"])
                .env_remove("TMUX")
                .env_remove("TMUX_TMPDIR")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Ok(mut child) = command.spawn() {
                drop(child.wait());
            }
        }
        // kill-server can leave the socket file. The daemon clears
        // `TMUX_TMPDIR`, so the file is under `/tmp/tmux-<uid>/`.
        if let Some(uid) = current_uid() {
            drop(fs::remove_file(format!(
                "/tmp/tmux-{uid}/{}",
                self.tmux_socket
            )));
        }
    }
}

fn current_uid() -> Option<u32> {
    if let Ok(text) = fs::read_to_string("/proc/self/status") {
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("Uid:") else {
                continue;
            };
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    let mut command = Command::new("id");
    command
        .arg("-u")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command.spawn().ok()?;
    let output = child.wait_with_output().ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn first_set(vars: &[(String, String)], names: &[&str]) -> Option<(String, String)> {
    for name in names {
        if let Some(value) = vars
            .iter()
            .find(|(key, value)| key == name && !value.is_empty())
        {
            return Some(value.clone());
        }
    }
    None
}

fn lookup(vars: &[(String, String)], name: &str) -> Option<String> {
    vars.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

fn scrubbed_env(vars: &[(String, String)]) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for (key, value) in vars {
        if is_scrubbed(key) {
            continue;
        }
        env.insert(key.clone(), value.clone());
    }
    env
}

fn is_scrubbed(key: &str) -> bool {
    DAEMON_TARGET_ENV.contains(&key)
        || CLOCK_ENV.contains(&key)
        || TMUX_ENV.contains(&key)
        || CREDENTIAL_ENV.contains(&key)
        || key == "TMUX_TMPDIR"
        || key == "OPENRIG_DIR"
        || key == "RIG_TMUX_SOCKET"
}

fn find_on_path(path: &str, name: &str) -> Option<PathBuf> {
    for dir in std::env::split_paths(path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn python3_on_path() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| find_on_path(&path.to_string_lossy(), "python3").is_some())
}

fn fresh_socket_name() -> String {
    // Short on purpose: macOS `sockaddr_un` is 104 bytes, and tmux puts this
    // name under a long temp directory. The pid prefix keeps this Drop from
    // killing a socket this process did not create. The same string is
    // `RIG_TMUX_SOCKET`, so the daemon never falls through to `rig-rs`.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    let seq = SOCKET_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("p{:x}{seq:x}{nanos:08x}", std::process::id())
}

fn harness_socket_owned(name: &str) -> bool {
    let prefix = format!("p{:x}", std::process::id());
    name.starts_with(&prefix) && name.len() > prefix.len()
}

fn write_shim(path: &Path) -> Result<(), Error> {
    let mut file = fs::File::create(path).map_err(|source| io_err("create tmux shim", source))?;
    file.write_all(SHIM.as_bytes())
        .map_err(|source| io_err("write tmux shim", source))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file
            .metadata()
            .map_err(|source| io_err("stat tmux shim", source))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).map_err(|source| io_err("chmod tmux shim", source))?;
    }
    Ok(())
}
