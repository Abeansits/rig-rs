//! One test per normalisation rule (AC4).

use std::collections::BTreeMap;
use std::path::PathBuf;

use xtask::diff::{NormRule, Observation, normalize_observation, normalize_text};

const ULID_A: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const ULID_B: &str = "01BX5ZZKBKACTAV9WEVGEMMVSX";

fn obs(stdout: &str, stderr: &str) -> Observation {
    Observation {
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
        exit: 0,
        json: None,
        files: BTreeMap::new(),
        tmux_argv: Vec::new(),
    }
}

#[test]
fn ulid_reuses_one_placeholder_for_the_same_value() {
    let text = format!("{ULID_A} {ULID_B} {ULID_A}");
    let out = normalize_text(&text, &[NormRule::Ulid], &[]).unwrap();
    assert_eq!(out, "<ULID:1> <ULID:2> <ULID:1>");
    let glued = format!("{ULID_A}X");
    let kept = normalize_text(&glued, &[NormRule::Ulid], &[]).unwrap();
    assert_eq!(kept, glued);
    let shared = normalize_observation(&obs(ULID_A, ULID_A), &[NormRule::Ulid], &[]).unwrap();
    assert_eq!(shared.stdout, "<ULID:1>");
    assert_eq!(shared.stderr, "<ULID:1>");
}

#[test]
fn timestamp_reuses_one_placeholder_for_the_same_value() {
    let text = "2024-10-11T10:38:00Z 2024-10-11T10:38:01+00:00 2024-10-11T10:38:00Z";
    let out = normalize_text(text, &[NormRule::Timestamp], &[]).unwrap();
    assert_eq!(out, "<TS:1> <TS:2> <TS:1>");
}

#[test]
fn timestamp_each_numbers_every_occurrence() {
    let text = "2024-10-11T10:38:00Z 2024-10-11T10:38:00.000Z";
    let out = normalize_text(text, &[NormRule::TimestampEach], &[]).unwrap();
    assert_eq!(out, "<TS:1> <TS:2>");
}

#[test]
fn abspath_masks_scratch_roots_and_temp_dirs() {
    let root = PathBuf::from("/tmp/scratch-root");
    let text = "/tmp/scratch-root/home/a /tmp/scratch-root/home/a /tmp/other/file";
    let out = normalize_text(text, &[NormRule::AbsPath], &[root]).unwrap();
    assert_eq!(out, "<PATH:1> <PATH:1> <PATH:2>");
}

#[test]
fn port_keeps_the_host_and_keys_the_placeholder_by_port() {
    let text = "127.0.0.1:8080 localhost:8080 127.0.0.1:9090";
    let out = normalize_text(text, &[NormRule::Port], &[]).unwrap();
    assert_eq!(
        out,
        "127.0.0.1:<PORT:1> localhost:<PORT:1> 127.0.0.1:<PORT:2>"
    );
}

#[test]
fn socketpath_is_masked_before_a_generic_temp_path() {
    let text = "/var/tmp/daemon.sock /tmp/b.sock /var/tmp/daemon.sock";
    let rules = [NormRule::SocketPath, NormRule::AbsPath];
    let out = normalize_text(text, &rules, &[]).unwrap();
    assert_eq!(out, "<SOCK:1> <SOCK:2> <SOCK:1>");
    assert!(!out.contains(".sock"));
}

#[test]
fn custom_rule_replaces_every_match_with_the_static_string() {
    let rule = NormRule::Custom(regex::Regex::new(r"secret-\d+").unwrap(), "<SECRET>");
    let out = normalize_text("secret-1 and secret-2", &[rule], &[]).unwrap();
    assert_eq!(out, "<SECRET> and <SECRET>");
}

#[test]
fn dash_getcwd_noise_is_stripped() {
    let text =
        "tmux has-session\nsh: 0: getcwd() failed: No such file or directory\nerror connecting\n";
    let out = normalize_text(text, &[NormRule::Getcwd], &[]).unwrap();
    assert_eq!(out, "tmux has-session\nerror connecting\n");
    let embedded = "tmux has-session -t '=one'\\nsh: 0: getcwd() failed: No such file or directory\\nerror connecting\\n";
    let out = normalize_text(embedded, &[NormRule::Getcwd], &[]).unwrap();
    assert_eq!(out, "tmux has-session -t '=one'\\nerror connecting\\n");
    let kept = normalize_text(text, &[], &[]).unwrap();
    assert!(kept.contains("getcwd() failed"), "{kept}");
}
