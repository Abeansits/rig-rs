//! JSON case files under `tests/diff/cases/`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::bind::{bind_transport, require_pe, tcp_target};
use super::types::{
    DiffCase, LoadedCase, Method, NormRule, Project, Side, Step, Transport, workspace_root,
};
use crate::error::{Error, io_err};

#[derive(Debug, Deserialize)]
struct CaseFile {
    id: String,
    steps: Vec<StepFile>,
    #[serde(default)]
    normalize: Vec<String>,
    #[serde(default)]
    expected_diff: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    needs: Option<String>,
    #[serde(default)]
    scenario: Option<String>,
    /// `tcp` or `tcp:<addr>` is rejected while the file is loaded.
    #[serde(default)]
    rs_transport: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum StepFile {
    Cli {
        argv: Vec<String>,
    },
    Http {
        method: String,
        path: String,
        #[serde(default)]
        body: Option<Value>,
        /// False keeps the response out of the observation. Default true.
        #[serde(default = "recorded")]
        record: bool,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
        body: String,
    },
    TmuxCalls,
    McpTools,
    EnsureRig {
        name: String,
    },
}

/// Load every `*.json` case in `tests/diff/cases`, sorted by id.
///
/// # Errors
///
/// Returns [`Error`] when a file is unreadable, a rig-rs transport is TCP, or a
/// parity-exception id is not a heading.
pub fn load_suite(exceptions: &Path) -> Result<Vec<LoadedCase>, Error> {
    let dir = workspace_root().join("tests/diff/cases");
    load_dir(&dir, exceptions)
}

/// Load every JSON case in `dir`.
///
/// # Errors
///
/// Same as [`load_suite`].
pub fn load_dir(dir: &Path, exceptions: &Path) -> Result<Vec<LoadedCase>, Error> {
    let mut paths = Vec::new();
    let entries =
        fs::read_dir(dir).map_err(|source| io_err(format!("read {}", dir.display()), source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_err("read case directory entry", source))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    let mut cases = Vec::new();
    for path in paths {
        cases.push(load_case(&path, exceptions)?);
    }
    cases.sort_by(|left, right| left.case.id.cmp(&right.case.id));
    Ok(cases)
}

/// Load one case file and reject a TCP binding for rig-rs.
///
/// # Errors
///
/// Returns [`Error::RsTcpRejected`] when `rs_transport` is TCP, before any daemon is dialed.
pub fn load_case(path: &Path, exceptions: &Path) -> Result<LoadedCase, Error> {
    let text = fs::read_to_string(path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    let file: CaseFile = serde_json::from_str(&text).map_err(|source| Error::Json {
        context: format!("parse {}", path.display()),
        source,
    })?;
    if let Some(spec) = file.rs_transport.as_deref()
        && (spec == "tcp" || spec.starts_with("tcp:"))
    {
        let addr = tcp_target(spec)?;
        let side = Side::Rs {
            bin: PathBuf::from("rig"),
        };
        bind_transport(&side, &Transport::Tcp(addr))?;
    }
    let expected_diff = match file.expected_diff.as_deref() {
        Some(text) if !text.is_empty() => Some(super::types::PeId::parse(text)?),
        _ => None,
    };
    let covers = require_pe(expected_diff.as_ref(), exceptions)?;
    let mut steps = Vec::new();
    for step in file.steps {
        steps.push(step_from(step)?);
    }
    let mut normalize = Vec::new();
    for name in &file.normalize {
        normalize.push(rule_from(name)?);
    }
    Ok(LoadedCase {
        case: DiffCase {
            id: file.id,
            steps,
            normalize,
            expected_diff,
            covers,
        },
        project: project_from(file.project.as_deref())?,
        needs: file.needs.filter(|text| !text.is_empty()),
        scenario: file.scenario.filter(|text| !text.is_empty()),
    })
}

const fn recorded() -> bool {
    true
}

fn step_from(step: StepFile) -> Result<Step, Error> {
    match step {
        StepFile::Cli { argv } => Ok(Step::Cli { argv }),
        StepFile::Http {
            method,
            path,
            body,
            record,
        } => Ok(Step::Http {
            method: Method::parse(&method)?,
            path,
            body,
            record,
        }),
        StepFile::ReadFile { path } => {
            if path.starts_with('/') {
                return Err(Error::Message(format!(
                    "ReadFile path {path} must stay relative to the hermetic HOME"
                )));
            }
            Ok(Step::ReadFile(PathBuf::from(path)))
        }
        StepFile::WriteFile { path, body } => {
            relative_path(&path).map(|path| Step::WriteFile { path, body })
        }
        StepFile::TmuxCalls => Ok(Step::TmuxCalls),
        StepFile::McpTools => Ok(Step::McpTools),
        StepFile::EnsureRig { name } => Ok(Step::EnsureRig { name }),
    }
}

fn relative_path(path: &str) -> Result<PathBuf, Error> {
    if path.starts_with('/') || path.split(['/', '\\']).any(|part| part == "..") {
        return Err(Error::Message(format!(
            "path {path} must stay relative to the hermetic HOME"
        )));
    }
    Ok(PathBuf::from(path))
}

fn rule_from(name: &str) -> Result<NormRule, Error> {
    match name {
        "ulid" => Ok(NormRule::Ulid),
        "timestamp" => Ok(NormRule::Timestamp),
        "abspath" => Ok(NormRule::AbsPath),
        "port" => Ok(NormRule::Port),
        "socketpath" => Ok(NormRule::SocketPath),
        "getcwd" => Ok(NormRule::Getcwd),
        "jsoncanon" => Ok(NormRule::JsonCanon),
        "qitem" => Ok(NormRule::Qitem),
        other => Err(Error::Message(format!(
            "unknown norm rule {other}; custom rules are constructed in code"
        ))),
    }
}

fn project_from(name: Option<&str>) -> Result<Project, Error> {
    match name {
        None | Some("full") => Ok(Project::Full),
        Some("exit") => Ok(Project::ExitCode),
        Some("help-names") => Ok(Project::HelpNames),
        Some("health-shape") => Ok(Project::HealthShape),
        Some("mcp-tool-names") => Ok(Project::McpToolNames),
        Some(other) => Err(Error::Message(format!("unknown project {other}"))),
    }
}
