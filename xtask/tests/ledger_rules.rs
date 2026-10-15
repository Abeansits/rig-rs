//! Ledger parser and one test per AC10 validation rule.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use xtask::diff::{PeId, pe_ids_in, workspace_root};
use xtask::ledger::{
    LedgerRow, LedgerStatus, check_case_titles, parse_csv, report, to_csv, validate, waiver_for,
};

fn row(path: &str, status: LedgerStatus) -> LedgerRow {
    LedgerRow {
        ts_path: path.to_owned(),
        slice: None,
        status,
        rust_test: None,
        pe_id: None,
        note: String::new(),
    }
}

fn pes() -> BTreeSet<PeId> {
    let mut set = BTreeSet::new();
    if let Ok(id) = PeId::parse("PE-001") {
        set.insert(id);
    }
    set
}

const HEADER: &str = "ts_path,slice,status,rust_test,pe_id,note\n";

#[test]
fn unknown_status_is_a_parse_error() {
    let csv = format!("{HEADER}\"a.test.ts\",\"\",\"bogus\",\"\",\"\",\"\"\n");
    let errors = parse_csv(&csv).unwrap_err();
    assert!(errors.iter().any(|error| error.contains("unknown status")));
}

#[test]
fn diverged_without_pe_id_is_an_error() {
    let rows = [row("a.test.ts", LedgerStatus::Diverged)];
    let errors = validate(&rows, &pes(), None);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("requires a pe_id"))
    );
}

#[test]
fn diverged_with_unknown_pe_id_is_an_error() {
    let mut sample = row("a.test.ts", LedgerStatus::Diverged);
    sample.pe_id = Some(PeId::parse("PE-999").unwrap());
    let errors = validate(std::slice::from_ref(&sample), &pes(), None);
    assert!(errors.iter().any(|error| error.contains("unknown pe_id")));
}

#[test]
fn ported_and_reused_require_rust_test() {
    for status in [LedgerStatus::Ported, LedgerStatus::Reused] {
        let errors = validate(&[row("a.test.ts", status)], &pes(), None);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("requires rust_test")),
            "{status:?}"
        );
    }
}

#[test]
fn waived_with_empty_note_is_an_error() {
    let errors = validate(&[row("a.test.ts", LedgerStatus::Waived)], &pes(), None);
    assert!(errors.iter().any(|error| error.contains("requires a note")));
}

#[test]
fn duplicate_ts_path_is_an_error() {
    let rows = [
        row("a.test.ts", LedgerStatus::Todo),
        row("a.test.ts", LedgerStatus::Todo),
    ];
    let errors = validate(&rows, &pes(), None);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("duplicate ts_path"))
    );
}

#[test]
fn case_row_with_a_listed_test_is_accepted() {
    let mut sample = row(
        "packages/daemon/test/a.test.ts > does the thing",
        LedgerStatus::Ported,
    );
    sample.rust_test = Some("does_the_thing".to_owned());
    let rows = [
        row("packages/daemon/test/a.test.ts", LedgerStatus::Todo),
        sample,
    ];
    let listed = BTreeSet::from(["does_the_thing".to_owned()]);
    assert_eq!(validate(&rows, &pes(), Some(&listed)), Vec::<String>::new());
}

#[test]
fn case_row_rust_test_must_be_listed() {
    let mut sample = row(
        "packages/daemon/test/a.test.ts > does the thing",
        LedgerStatus::Ported,
    );
    sample.rust_test = Some("missing".to_owned());
    let rows = [
        row("packages/daemon/test/a.test.ts", LedgerStatus::Todo),
        sample,
    ];
    let listed = BTreeSet::from(["present".to_owned()]);
    let errors = validate(&rows, &pes(), Some(&listed));
    assert!(
        errors
            .iter()
            .any(|error| error.contains("not in `cargo test -- --list`"))
    );
}

#[test]
fn case_row_without_rust_test_is_an_error() {
    let rows = [
        row("a.test.ts", LedgerStatus::Todo),
        row("a.test.ts > a case", LedgerStatus::Ported),
    ];
    let errors = validate(&rows, &pes(), None);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("requires rust_test"))
    );
}

#[test]
fn malformed_case_separator_is_an_error() {
    let mut sample = row("a.test.ts>no-spaces", LedgerStatus::Ported);
    sample.rust_test = Some("present".to_owned());
    let rows = [row("a.test.ts", LedgerStatus::Todo), sample];
    let errors = validate(&rows, &pes(), None);
    assert!(errors.iter().any(|error| error.contains("must look like")));
}

#[test]
fn case_row_parent_must_be_a_file_row() {
    let mut sample = row("missing.test.ts > a case", LedgerStatus::Ported);
    sample.rust_test = Some("present".to_owned());
    let listed = BTreeSet::from(["present".to_owned()]);
    let errors = validate(std::slice::from_ref(&sample), &pes(), Some(&listed));
    assert!(
        errors
            .iter()
            .any(|error| error.contains("not a file-level ledger row"))
    );
}

#[test]
fn rust_test_must_appear_in_the_cargo_list() {
    let mut sample = row("a.test.ts", LedgerStatus::Ported);
    sample.rust_test = Some("missing::test".to_owned());
    let listed = BTreeSet::from(["present::test".to_owned()]);
    let errors = validate(std::slice::from_ref(&sample), &pes(), Some(&listed));
    assert!(
        errors
            .iter()
            .any(|error| error.contains("not in `cargo test -- --list`"))
    );
}

#[test]
fn round_trip_preserves_a_note_that_contains_a_comma() {
    let mut sample = row("a.test.ts", LedgerStatus::Waived);
    sample.note = "gateway, slack".to_owned();
    let parsed = parse_csv(&to_csv(std::slice::from_ref(&sample))).unwrap();
    assert_eq!(parsed, vec![sample]);
}

#[test]
fn waiver_rules_leave_in_scope_names_as_todo() {
    assert!(
        waiver_for("packages/daemon/test/gateway/slack.test.ts")
            .unwrap()
            .is_some()
    );
    assert!(
        waiver_for("packages/cli/test/openrig-compat.test.ts")
            .unwrap()
            .is_some()
    );
    assert!(
        waiver_for("packages/daemon/test/skill-discovery.test.ts")
            .unwrap()
            .is_none()
    );
    assert!(
        waiver_for("packages/daemon/test/fleet-compose.test.ts")
            .unwrap()
            .is_none()
    );
    assert!(
        waiver_for("packages/daemon/test/startup-proof.test.ts")
            .unwrap()
            .is_none()
    );
    assert!(
        waiver_for("packages/daemon/test/proof-command.test.ts")
            .unwrap()
            .is_some()
    );
}

#[test]
fn workspace_root_walks_up_from_the_process() {
    let root = workspace_root();
    assert!(
        root.join("parity/ledger.csv").is_file(),
        "{}",
        root.display()
    );
    assert!(root.join("xtask/Cargo.toml").is_file());
    assert_ne!(root, PathBuf::from(env!("CARGO_MANIFEST_DIR")));
}

#[test]
fn slice2_titles_match_the_pin_when_openrig_dir_is_set() {
    let Ok(dir) = std::env::var("OPENRIG_DIR") else {
        return;
    };
    let root = workspace_root();
    let text = fs::read_to_string(root.join("parity/ledger.csv")).unwrap();
    let rows = parse_csv(&text).unwrap();
    let errors = check_case_titles(&rows, std::path::Path::new(&dir));
    let ours: Vec<_> = errors
        .into_iter()
        .filter(|err| {
            err.contains("claude-runtime-adapter")
                || err.contains("claude-warning-footer")
                || err.contains("claude-interactive-launch")
                || err.contains("native-resume-probe")
                || err.contains("native-claude-delivery")
        })
        .collect();
    assert_eq!(ours, Vec::<String>::new(), "{ours:?}");
}

#[test]
fn seeded_ledger_is_check_green() {
    let root = workspace_root();
    let text = fs::read_to_string(root.join("parity/ledger.csv")).unwrap();
    let rows = parse_csv(&text).unwrap();
    assert_eq!(rows.len(), 1938);
    for row in &rows {
        match row.status {
            LedgerStatus::Todo => {
                assert!(row.rust_test.is_none(), "{}", row.ts_path);
                assert!(row.pe_id.is_none(), "{}", row.ts_path);
            }
            LedgerStatus::Waived => {
                assert!(!row.note.trim().is_empty(), "{}", row.ts_path);
                assert!(row.rust_test.is_none(), "{}", row.ts_path);
                assert!(row.pe_id.is_none(), "{}", row.ts_path);
            }
            LedgerStatus::Ported | LedgerStatus::Reused => {
                assert!(
                    !row.rust_test.as_deref().unwrap_or("").trim().is_empty(),
                    "{}",
                    row.ts_path
                );
                assert!(row.pe_id.is_none(), "{}", row.ts_path);
            }
            LedgerStatus::Diverged => assert!(row.pe_id.is_some(), "{}", row.ts_path),
        }
    }
    let markdown = fs::read_to_string(root.join("design/parity-exceptions.md")).unwrap();
    let pe_ids = pe_ids_in(&markdown).into_iter().collect();
    assert_eq!(validate(&rows, &pe_ids, None), Vec::<String>::new());
    let printed = report(&rows);
    assert!(printed.contains("| todo |"));
    assert!(printed.contains("| waived |"));
    assert!(printed.contains("(unassigned)"));
}

#[test]
fn bogus_case_name_fails_when_the_ts_checkout_is_present() {
    let dir = tempfile::tempdir().expect("temp");
    let rel = "packages/daemon/test/sample.test.ts";
    let file = dir.path().join(rel);
    fs::create_dir_all(file.parent().expect("parent")).expect("dir");
    fs::write(
        &file,
        "it(`${mode}: preserves exact literal names and session-id reads`, () => {});\n\
         it.each(stores)(\"%s retains a native config symlink and its target permissions on successful writes\", () => {});\n\
         it.each<Kind>([\"a\"])(\"direct detail %s still records failure\", () => {});\n\
         it(\"tmux available -> pass\", () => {});\n\
         it(\"before regex\", () => { x.toMatch(/^a '[^']+' b$/); });\n\
         it(\"after a regex literal\", () => {});\n",
    )
    .expect("write");
    let errors = check_case_titles(&[case(rel, "rewrites dot and colon")], dir.path());
    assert!(
        errors
            .iter()
            .any(|error| error.contains("rewrites dot and colon")),
        "{errors:?}"
    );
    assert_eq!(
        check_case_titles(
            &[case(
                rel,
                "preserves exact literal names and session-id reads"
            )],
            dir.path()
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        check_case_titles(
            &[case(
                rel,
                "%s retains a native config symlink and its target permissions on successful writes"
            )],
            dir.path()
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        check_case_titles(
            &[case(rel, "direct detail %s still records failure")],
            dir.path()
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        check_case_titles(&[case(rel, "after a regex literal")], dir.path()),
        Vec::<String>::new()
    );
    let substring = check_case_titles(&[case(rel, "pass")], dir.path());
    assert!(
        substring.iter().any(|error| error.contains("pass")),
        "{substring:?}"
    );
    let missing = check_case_titles(
        &[case("packages/missing.test.ts", "tmux available -> pass")],
        dir.path(),
    );
    assert!(
        missing.iter().any(|error| error.contains("cannot read")),
        "{missing:?}"
    );
}

fn case(parent: &str, name: &str) -> LedgerRow {
    LedgerRow {
        ts_path: format!("{parent} > {name}"),
        slice: Some("1b".to_owned()),
        status: LedgerStatus::Ported,
        rust_test: Some("sample".to_owned()),
        pe_id: None,
        note: String::new(),
    }
}
