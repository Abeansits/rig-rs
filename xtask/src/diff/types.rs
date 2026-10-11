//! Public harness types. Stable for the parity ledger and for QA.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{Map, Value};
use tempfile::TempDir;

use crate::error::Error;

/// Which implementation a case runs against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    /// TypeScript `OpenRig` v0.6.9, launched with `node` against a package checkout.
    Ts {
        /// `node` binary.
        node: PathBuf,
        /// Checkout root whose `package.json` version is `0.6.9`.
        pkg: PathBuf,
    },
    /// The `rig` binary built from this repo.
    Rs {
        /// Path to the `rig` executable.
        bin: PathBuf,
    },
}

/// How the harness reaches a daemon.
///
/// rig-rs is [`Transport::Uds`]. TypeScript `OpenRig` is [`Transport::Tcp`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// Unix socket, normally `$RIG_HOME/run/daemon.sock`.
    Uds(PathBuf),
    /// TCP `127.0.0.1:<port>`. Rejected at load time for [`Side::Rs`].
    Tcp(SocketAddr),
}

/// Scratch home, private tmux socket, and a PATH prefix that puts the tmux shim first.
///
/// Dropping the value deletes the scratch directory and asks the private tmux
/// server to exit. The real `$HOME` and the default tmux socket are never written.
#[derive(Debug)]
pub struct HermeticHome {
    /// Scratch root. Deleted on drop.
    pub root: TempDir,
    /// `HOME` for child processes.
    pub home: PathBuf,
    /// `tmux -L` name, unique for this value.
    pub tmux_socket: String,
    /// Directories prepended to `PATH`. The tmux shim lives in the first one.
    pub path_prepend: Vec<PathBuf>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) tmux_log: PathBuf,
    pub(crate) real_tmux: Option<PathBuf>,
    pub(crate) shim: PathBuf,
}

/// One field of a canonical [`Observation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObsField {
    /// Process exit status.
    Exit,
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
    /// Last JSON body.
    Json,
    /// Files read by [`Step::ReadFile`].
    Files,
    /// tmux argv recorded by the shim.
    TmuxArgv,
}

impl ObsField {
    /// Every field a canonical observation carries.
    pub const ALL: [Self; 6] = [
        Self::Exit,
        Self::Stdout,
        Self::Stderr,
        Self::Json,
        Self::Files,
        Self::TmuxArgv,
    ];

    /// JSON key in a canonical observation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exit => "exit",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Json => "json",
            Self::Files => "files",
            Self::TmuxArgv => "tmux_argv",
        }
    }

    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "exit" => Ok(Self::Exit),
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "json" => Ok(Self::Json),
            "files" => Ok(Self::Files),
            "tmux_argv" => Ok(Self::TmuxArgv),
            other => Err(Error::Message(format!(
                "unknown observation field {other}; expected exit, stdout, stderr, json, files, or tmux_argv"
            ))),
        }
    }
}

/// One differential case.
#[derive(Debug, Clone)]
pub struct DiffCase {
    /// Stable id. Also the golden file stem.
    pub id: String,
    /// Script of commands, HTTP calls, file reads, and tmux-log collection.
    pub steps: Vec<Step>,
    /// Masks applied to both sides before comparison.
    pub normalize: Vec<NormRule>,
    /// Exception id, when this case is an expected diff.
    pub expected_diff: Option<PeId>,
    /// Fields [`Self::expected_diff`] may differ on. Empty when that is unset.
    pub covers: Vec<ObsField>,
}

/// A single action inside a [`DiffCase`].
#[derive(Debug, Clone)]
pub enum Step {
    /// Run `rig` with these arguments (`--version`, `--help`, …).
    Cli {
        /// Arguments after the binary name.
        argv: Vec<String>,
    },
    /// HTTP call against the side's daemon transport.
    Http {
        /// Verb.
        method: Method,
        /// Absolute path such as `/healthz`.
        path: String,
        /// JSON body, when the verb carries one.
        body: Option<Value>,
    },
    /// Read a file relative to the hermetic `HOME`.
    ReadFile(PathBuf),
    /// Write `body` to a path relative to the hermetic `HOME`.
    WriteFile {
        /// Relative path.
        path: PathBuf,
        /// File contents.
        body: String,
    },
    /// Copy the tmux shim's argv log into the observation.
    TmuxCalls,
}

/// HTTP verb used by [`Step::Http`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
}

impl Method {
    /// Token used on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "PATCH" => Ok(Self::Patch),
            "DELETE" => Ok(Self::Delete),
            other => Err(Error::Message(format!("unsupported HTTP method {other}"))),
        }
    }
}

/// What one side produced for a case.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// Standard output, concatenated across CLI steps.
    pub stdout: String,
    /// Standard error, concatenated across CLI steps.
    pub stderr: String,
    /// Exit status of the last CLI or HTTP step. `0` when the case has neither.
    pub exit: i32,
    /// Last JSON body, when a step produced one.
    pub json: Option<Value>,
    /// Files read by [`Step::ReadFile`], keyed by the (possibly masked) path.
    pub files: BTreeMap<PathBuf, String>,
    /// tmux invocations recorded by the shim, in order.
    pub tmux_argv: Vec<Vec<String>>,
}

impl Observation {
    pub(crate) fn empty() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            exit: 0,
            json: None,
            files: BTreeMap::new(),
            tmux_argv: Vec::new(),
        }
    }
}

/// Mask applied to volatile text. The same raw value becomes the same placeholder
/// everywhere inside one observation.
#[derive(Debug, Clone)]
pub enum NormRule {
    /// Crockford base32 [`ULID`](https://github.com/ulid/spec) (26 characters).
    Ulid,
    /// RFC 3339 timestamps.
    Timestamp,
    /// Absolute paths under the scratch root or a system temp directory.
    AbsPath,
    /// TCP ports in `host:port` form (`127.0.0.1`, `localhost`, `::1`).
    Port,
    /// Paths that end in `.sock`.
    SocketPath,
    /// Caller-supplied replacement. Every match becomes the static string.
    Custom(Regex, &'static str),
}

/// Result of comparing the two sides after normalisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffVerdict {
    /// Canonical observations are byte-identical.
    Equal,
    /// The Rust side matches the exception snapshot, and the only differences
    /// are fields that exception covers.
    ExpectedDiff(PeId),
    /// Observations differ and the case did not name an exception.
    /// The string is a unified diff.
    Unexpected(String),
}

/// Id of a heading in `design/parity-exceptions.md`, such as `PE-001`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeId(String);

impl PeId {
    /// Parse `PE-` followed by uppercase letters or digits.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BadPe`] when the text is not that shape.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let chars_ok = text
            .chars()
            .skip(3)
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if text.starts_with("PE-") && text.len() > 3 && chars_ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(Error::BadPe(text.to_owned()))
        }
    }

    /// Borrow the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PeId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// How much of an observation a case compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Project {
    /// The whole observation.
    Full,
    /// Exit code only. Used by `rig --version`, whose version string differs by design.
    ExitCode,
    /// Sorted command names from `rig --help`.
    HelpNames,
    /// Top-level `/healthz` keys plus the `status` string.
    HealthShape,
}

/// A case plus the comparison policy loaded from its JSON file.
#[derive(Debug, Clone)]
pub struct LoadedCase {
    /// The case itself.
    pub case: DiffCase,
    /// Projection applied after normalisation.
    pub project: Project,
    /// When set, the case is listed and not executed. The text is `slice 0` or `slice 1`.
    pub needs: Option<String>,
    /// Test-system scenario this probe stands in for, when there is one.
    pub scenario: Option<String>,
}

/// Canonical JSON text used for byte comparison and unified diffs.
///
/// # Errors
///
/// Returns [`Error::Json`] if the observation cannot be serialised.
pub fn canonical(obs: &Observation) -> Result<String, Error> {
    let mut files = Vec::new();
    for (path, body) in &obs.files {
        let mut entry = Map::new();
        entry.insert("path".to_owned(), Value::String(path.display().to_string()));
        entry.insert("body".to_owned(), Value::String(body.clone()));
        files.push(Value::Object(entry));
    }
    let mut map = Map::new();
    map.insert("exit".to_owned(), Value::from(obs.exit));
    map.insert("stdout".to_owned(), Value::String(obs.stdout.clone()));
    map.insert("stderr".to_owned(), Value::String(obs.stderr.clone()));
    map.insert("json".to_owned(), obs.json.clone().unwrap_or(Value::Null));
    map.insert("files".to_owned(), Value::Array(files));
    map.insert(
        "tmux_argv".to_owned(),
        serde_json::to_value(&obs.tmux_argv).map_err(|source| Error::Json {
            context: "tmux argv".to_owned(),
            source,
        })?,
    );
    let mut text =
        serde_json::to_string_pretty(&Value::Object(map)).map_err(|source| Error::Json {
            context: "canonical observation".to_owned(),
            source,
        })?;
    text.push('\n');
    Ok(text)
}

/// Workspace root, resolved when the process runs.
///
/// Walks up from the working directory, then from the executable, until it finds
/// the checkout that contains `xtask/Cargo.toml` and `parity/ledger.csv`.
#[must_use]
pub fn workspace_root() -> PathBuf {
    std::env::current_dir()
        .ok()
        .as_deref()
        .and_then(find_workspace)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .as_deref()
                .and_then(find_workspace)
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

fn find_workspace(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    if !current.is_dir() {
        current.pop();
    }
    loop {
        if is_workspace(&current) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn is_workspace(dir: &Path) -> bool {
    dir.join("xtask/Cargo.toml").is_file() && dir.join("parity/ledger.csv").is_file()
}

/// Git tag the harness runs on the TypeScript side.
pub const OPENRIG_TAG: &str = "v0.6.9";

/// Commit the `v0.6.9` tag points at.
pub const OPENRIG_COMMIT: &str = "4b48ca21a9bd072aa05a08b3da6d9c0708e093c5";
