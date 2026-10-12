//! `RigSpec` parse and validation. Golden messages live in insta snapshots.

use crewmux_spec::{SpecError, parse};
use crewmux_types::RuntimeKind;

const TWO: &str = "\
name: pair
seats:
  - name: one
    runtime: stub
    cwd: /tmp/one
    model: small
  - name: two
    runtime: stub
";

#[test]
fn valid_two_seat_spec() {
    let spec = parse(TWO).expect("spec");
    assert_eq!(spec.name.as_str(), "pair");
    assert_eq!(spec.seats.len(), 2);
    assert_eq!(spec.seats[0].name.as_str(), "one");
    assert_eq!(spec.seats[0].runtime, RuntimeKind::Stub);
    assert_eq!(
        spec.seats[0]
            .cwd
            .as_deref()
            .map(|path| path.display().to_string()),
        Some("/tmp/one".to_owned())
    );
    assert_eq!(spec.seats[0].model.as_deref(), Some("small"));
    assert_eq!(spec.seats[1].name.as_str(), "two");
    assert!(spec.seats[1].cwd.is_none());
}

#[test]
fn unknown_field_reports_line_and_column() {
    let text = "\
name: pair
extra: true
seats:
  - name: one
    runtime: stub
";
    let err = parse(text).expect_err("unknown field");
    assert_eq!(err.line, 2);
    assert_eq!(err.col, 1);
    assert!(err.message.contains("extra"), "{err}");
    insta::assert_snapshot!(golden(&err));
}

#[test]
fn duplicate_key_reports_line_and_column() {
    let text = "\
name: pair
name: other
seats:
  - name: one
    runtime: stub
";
    let err = parse(text).expect_err("duplicate key");
    assert_eq!(err.line, 2);
    assert!(err.col >= 1, "{err}");
    assert!(
        err.message.to_ascii_lowercase().contains("duplicate"),
        "{err}"
    );
    insta::assert_snapshot!(golden(&err));
}

#[test]
fn runtime_pi_is_rejected_with_location() {
    let text = "\
name: pair
seats:
  - name: one
    runtime: pi
";
    let err = parse(text).expect_err("pi");
    assert_eq!(err.line, 4);
    assert!(err.col >= 1, "{err}");
    assert!(err.message.contains("pi"), "{err}");
    insta::assert_snapshot!(golden(&err));
}

#[test]
fn duplicate_seat_name_is_rejected() {
    let text = "\
name: pair
seats:
  - name: one
    runtime: stub
  - name: one
    runtime: stub
";
    let err = parse(text).expect_err("duplicate seat");
    assert!(err.message.contains("duplicate seat"), "{err}");
    assert_eq!(err.path, "seats[1].name");
}

#[test]
fn names_longer_than_256_bytes_are_rejected() {
    let name = "n".repeat(257);
    let text = format!("name: {name}\nseats:\n  - name: one\n    runtime: stub\n");
    let err = parse(&text).expect_err("long rig");
    assert!(err.message.contains("256"), "{err}");
    assert!(!err.message.contains(&name));
    let seat = format!("name: pair\nseats:\n  - name: {name}\n    runtime: stub\n");
    let err = parse(&seat).expect_err("long seat");
    assert!(err.message.contains("256"), "{err}");
    assert!(!err.message.contains(&name));
}

#[test]
fn seat_names_reject_tmux_format_characters() {
    let err = parse("name: pair\nseats:\n  - name: \"#{pane_id}\"\n    runtime: stub\n")
        .expect_err("format");
    assert!(err.message.contains("[A-Za-z0-9_-]"), "{err:?}");
    let err = parse("name: pair\nseats:\n  - name: one.two\n    runtime: stub\n").expect_err("dot");
    assert!(err.message.contains("[A-Za-z0-9_-]"), "{err:?}");
}

fn golden(err: &SpecError) -> String {
    format!(
        "{} at line {} column {} ({})",
        err.message, err.line, err.col, err.path
    )
}
