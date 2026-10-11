//! `cargo xtask` entry point for the diff harness and the parity ledger.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use xtask::diff::{
    exit_code, find_node, format_report, load_suite, locate_openrig, self_reports, ts_reports,
    workspace_root,
};
use xtask::ledger::{parse_csv, parse_test_list, report, seed_csv, validate};
use xtask::{Error, err, io_err, out};

const HELP: &str = "\
xtask diff [--self] [--update] [--case ID] [--bin PATH]
xtask ledger seed [--openrig DIR]
xtask ledger check
xtask ledger report
xtask lint claude-json
xtask lint window-size

diff exits 0 when every verdict is Equal or ExpectedDiff, 1 on Unexpected or a
stale exception, and 2 on a harness error. --update rewrites only TS goldens.
--self compares the rig binary to itself and does not need Node.
lint exits 1 when a source check fails.
";

fn main() {
    let code = match dispatch() {
        Ok(code) => code,
        Err(error) => {
            err(&error.to_string());
            2
        }
    };
    exit_process(code);
}

#[expect(
    clippy::exit,
    reason = "process status is the diff and ledger contract"
)]
fn exit_process(code: i32) -> ! {
    std::process::exit(code);
}

fn dispatch() -> Result<i32, Error> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("diff") => cmd_diff(&mut args),
        Some("ledger") => cmd_ledger(&mut args),
        Some("lint") => cmd_lint(&mut args),
        Some("--help" | "-h") | None => {
            out(HELP);
            Ok(0)
        }
        Some(other) => Err(Error::Usage(format!("unknown command {other}\n{HELP}"))),
    }
}

struct DiffFlags {
    case: Option<String>,
    self_mode: bool,
    update: bool,
    bin: Option<PathBuf>,
}

fn cmd_diff(args: &mut impl Iterator<Item = String>) -> Result<i32, Error> {
    let flags = parse_diff_flags(args)?;
    if flags.update && flags.self_mode {
        return Err(Error::Usage(
            "--update rewrites TS-side goldens only and is not valid with --self".to_owned(),
        ));
    }
    let root = workspace_root();
    let exceptions = root.join("design/parity-exceptions.md");
    let mut cases = load_suite(&exceptions)?;
    if let Some(id) = &flags.case {
        cases.retain(|case| &case.case.id == id);
        if cases.is_empty() {
            return Err(Error::Message(format!("no diff case named {id}")));
        }
    }
    let bin = match flags.bin {
        Some(path) => path,
        None => default_rig_bin()?,
    };
    let reports = if flags.self_mode {
        self_reports(&cases, &bin)?
    } else {
        let pkg = locate_openrig()?;
        let node = find_node()?;
        ts_reports(&cases, &bin, &node, &pkg, flags.update)?
    };
    out(&format_report(&reports));
    Ok(exit_code(&reports))
}

fn parse_diff_flags(args: &mut impl Iterator<Item = String>) -> Result<DiffFlags, Error> {
    let mut flags = DiffFlags {
        case: None,
        self_mode: false,
        update: false,
        bin: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--self" => flags.self_mode = true,
            "--update" => flags.update = true,
            "--case" => flags.case = Some(need_value(args, "--case")?),
            "--bin" => flags.bin = Some(PathBuf::from(need_value(args, "--bin")?)),
            other => return Err(Error::Usage(format!("unknown diff flag {other}\n{HELP}"))),
        }
    }
    Ok(flags)
}

fn cmd_lint(args: &mut impl Iterator<Item = String>) -> Result<i32, Error> {
    let root = workspace_root();
    match args.next().as_deref() {
        Some("claude-json") => match xtask::lint_claude_json(&root) {
            Ok(()) => {
                out("lint-claude-json ok");
                Ok(0)
            }
            Err(error) => {
                err(&error.to_string());
                Ok(1)
            }
        },
        Some("window-size") => match xtask::lint_window_size(&root) {
            Ok(()) => {
                out("lint-window-size ok");
                Ok(0)
            }
            Err(error) => {
                err(&error.to_string());
                Ok(1)
            }
        },
        other => Err(Error::Usage(format!(
            "unknown lint command {other:?}\n{HELP}"
        ))),
    }
}

fn cmd_ledger(args: &mut impl Iterator<Item = String>) -> Result<i32, Error> {
    match args.next().as_deref() {
        Some("seed") => cmd_seed(args),
        Some("check") => cmd_check(),
        Some("report") => cmd_report(),
        other => Err(Error::Usage(format!(
            "unknown ledger command {other:?}\n{HELP}"
        ))),
    }
}

fn cmd_seed(args: &mut impl Iterator<Item = String>) -> Result<i32, Error> {
    let mut openrig = env::var_os("OPENRIG_DIR").map(PathBuf::from);
    while let Some(arg) = args.next() {
        if arg == "--openrig" {
            openrig = Some(PathBuf::from(need_value(args, "--openrig")?));
        } else {
            return Err(Error::Usage(format!("unknown seed flag {arg}\n{HELP}")));
        }
    }
    let Some(openrig) = openrig else {
        return Err(Error::Message(
            "ledger seed needs OPENRIG_DIR or --openrig pointing at OpenRig v0.6.9".to_owned(),
        ));
    };
    let csv = seed_csv(&openrig)?;
    let path = workspace_root().join("parity/ledger.csv");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| io_err("create parity dir", source))?;
    }
    fs::write(&path, csv).map_err(|source| io_err(format!("write {}", path.display()), source))?;
    out(&format!("wrote {}", path.display()));
    Ok(0)
}

fn cmd_check() -> Result<i32, Error> {
    let path = workspace_root().join("parity/ledger.csv");
    let text = fs::read_to_string(&path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    let rows = match parse_csv(&text) {
        Ok(rows) => rows,
        Err(errors) => {
            for error in &errors {
                err(error);
            }
            return Ok(1);
        }
    };
    let pe_ids = pe_set()?;
    let listed = if rows.iter().any(|row| row.rust_test.is_some()) {
        Some(cargo_test_list()?)
    } else {
        None
    };
    let errors = validate(&rows, &pe_ids, listed.as_ref());
    if errors.is_empty() {
        out(&format!("ledger ok ({} rows)", rows.len()));
        Ok(0)
    } else {
        for error in &errors {
            err(error);
        }
        Ok(1)
    }
}

fn cmd_report() -> Result<i32, Error> {
    let rows = read_ledger()?;
    out(&report(&rows));
    Ok(0)
}

fn read_ledger() -> Result<Vec<xtask::ledger::LedgerRow>, Error> {
    let path = workspace_root().join("parity/ledger.csv");
    let text = fs::read_to_string(&path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    parse_csv(&text).map_err(|errors| Error::Message(errors.join("\n")))
}

fn pe_set() -> Result<BTreeSet<xtask::diff::PeId>, Error> {
    let path = workspace_root().join("design/parity-exceptions.md");
    let ids = xtask::diff::pe_ids_in(
        &fs::read_to_string(&path)
            .map_err(|source| io_err(format!("read {}", path.display()), source))?,
    );
    Ok(ids.into_iter().collect())
}

fn cargo_test_list() -> Result<BTreeSet<String>, Error> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut command = Command::new(cargo);
    command
        .current_dir(workspace_root())
        .args(["test", "--workspace", "--locked", "--", "--list"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|source| io_err("spawn cargo test --list", source))?;
    let output = child
        .wait_with_output()
        .map_err(|source| io_err("wait for cargo test --list", source))?;
    if !output.status.success() {
        return Err(Error::Message(format!(
            "cargo test --list failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(parse_test_list(&String::from_utf8_lossy(&output.stdout)))
}

fn default_rig_bin() -> Result<PathBuf, Error> {
    if let Some(path) = env::var_os("RIG_BIN") {
        return Ok(PathBuf::from(path));
    }
    let root = workspace_root();
    for profile in ["debug", "release"] {
        let path = root.join("target").join(profile).join("rig");
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(Error::Message(
        "rig binary not found; run `cargo build -p rig` or set RIG_BIN".to_owned(),
    ))
}

fn need_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, Error> {
    args.next()
        .ok_or_else(|| Error::Usage(format!("{flag} needs a value")))
}
