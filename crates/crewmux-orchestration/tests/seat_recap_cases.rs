//! Seat recap store and publication cases from the TypeScript files.
#![expect(clippy::expect_used, reason = "fixture setup failures abort the case")]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_orchestration::context_packs::{
    ContractFinding, RecapFault, RecapPointer, list_recap_chain, resolve_authored_recap_pointer,
    set_recap_fault_for_tests, validate_recap_contract, write_seat_recap,
};
use sha2::{Digest, Sha256};

const GOOD_RECAP: &str = "## Recent Decisions\nWe chose X because Y outweighed Z at the time.\nUNVERIFIED: the Z cost figure came second-hand from the ops channel.\n## Open Threads\nDerive the live count with `rig queue list --mine` rather than trusting this snapshot.";

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rig-recap-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temp");
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        set_recap_fault_for_tests(None);
        drop(fs::remove_dir_all(&self.0));
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            push_hex(&mut out, *byte);
            out
        })
}

fn push_hex(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hi = usize::from(byte / 16);
    let lo = usize::from(byte % 16);
    if let (Some(hi), Some(lo)) = (HEX.get(hi), HEX.get(lo)) {
        out.push(char::from(*hi));
        out.push(char::from(*lo));
    }
}

fn body(path: &std::path::Path) -> String {
    fs::read_to_string(path).expect("read")
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn retains_a_recap_containing_the_blank_heading_boundary() {
    let headings = [
        "##",
        "##\n### Child\nchild prose",
        "###",
        "#",
        "#\n### Next\n## Next",
    ];
    let recorded = include_str!("fixtures/labels.jsonl");
    let line = recorded
        .lines()
        .find(|line| line.contains("good-recap"))
        .expect("row");
    let sha = line
        .split("\"sha256\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("sha");
    assert_eq!(sha, sha256_hex(GOOD_RECAP.as_bytes()));
    for heading in headings {
        let tmp = Tmp::new();
        write_seat_recap(&tmp.0, GOOD_RECAP, 1).expect("first");
        let content =
            format!("## Recent Decisions\nOwned decision\n{heading}\nUnaddressed prose\n");
        write_seat_recap(&tmp.0, &content, 2).expect("second");
        assert_eq!(body(&tmp.0.join("RECAP.md")), content);
        let chain = list_recap_chain(&tmp.0);
        assert_eq!(chain.len(), 1);
        assert_eq!(body(&chain[0]), GOOD_RECAP);
    }
}

#[test]
fn refuses_a_named_unaddressable_heading_after_a_blank_boundary() {
    let tmp = Tmp::new();
    write_seat_recap(&tmp.0, GOOD_RECAP, 1).expect("first");
    let err = write_seat_recap(&tmp.0, "##\n### ???\nunaddressable title", 2).expect_err("bad");
    assert!(err.message.contains("unaddressable-header"));
    assert_eq!(body(&tmp.0.join("RECAP.md")), GOOD_RECAP);
    assert_eq!(list_recap_chain(&tmp.0), Vec::<PathBuf>::new());
}

#[test]
fn first_write_creates_recap_and_second_supersedes_byte_preserved() {
    let tmp = Tmp::new();
    write_seat_recap(&tmp.0, "## Recent Decisions\nfirst era", 1000).expect("first");
    write_seat_recap(&tmp.0, GOOD_RECAP, 2000).expect("second");
    assert_eq!(body(&tmp.0.join("RECAP.md")), GOOD_RECAP);
    let chain = list_recap_chain(&tmp.0);
    assert_eq!(chain.len(), 1);
    assert_eq!(body(&chain[0]), "## Recent Decisions\nfirst era");
    assert!(chain[0].starts_with(&tmp.0));
}

#[test]
fn three_eras_list_oldest_first() {
    let tmp = Tmp::new();
    for (index, era) in ["one", "two", "three"].iter().enumerate() {
        let stamp = u64::try_from(index).expect("index").saturating_add(1);
        write_seat_recap(&tmp.0, &format!("## Recent Decisions\n{era}"), stamp).expect("era");
    }
    assert!(body(&tmp.0.join("RECAP.md")).contains("three"));
    let chain = list_recap_chain(&tmp.0);
    assert_eq!(
        chain.iter().map(|path| body(path)).collect::<Vec<_>>(),
        ["## Recent Decisions\none", "## Recent Decisions\ntwo"]
    );
}

#[test]
fn same_millisecond_supersessions_lose_nothing() {
    let tmp = Tmp::new();
    write_seat_recap(&tmp.0, "## Decisions\nv1", 5000).expect("v1");
    write_seat_recap(&tmp.0, "## Decisions\nv2", 7777).expect("v2");
    write_seat_recap(&tmp.0, "## Decisions\nv3", 7777).expect("v3");
    let chain = list_recap_chain(&tmp.0);
    assert_eq!(chain.len(), 2);
    let bodies: Vec<_> = chain.iter().map(|path| body(path)).collect();
    assert!(bodies.contains(&"## Decisions\nv1".to_owned()));
    assert!(bodies.contains(&"## Decisions\nv2".to_owned()));
    assert_eq!(body(&tmp.0.join("RECAP.md")), "## Decisions\nv3");
}

#[test]
fn a_write_that_fails_the_gate_leaves_current_and_chain_untouched() {
    let tmp = Tmp::new();
    write_seat_recap(&tmp.0, "## Decisions\nstanding era", 1).expect("first");
    assert!(write_seat_recap(&tmp.0, "## Same\na\n## Same\nb", 2).is_err());
    assert_eq!(body(&tmp.0.join("RECAP.md")), "## Decisions\nstanding era");
    assert_eq!(list_recap_chain(&tmp.0), Vec::<PathBuf>::new());
}

#[test]
fn unaddressable_write_is_rejected_loud() {
    let tmp = Tmp::new();
    let err = write_seat_recap(&tmp.0, "## Same\na\n## Same\nb", 1).expect_err("dup");
    let lower = err.message.to_ascii_lowercase();
    assert!(lower.contains("duplicate") || lower.contains("addressab"));
}

#[test]
fn a_contract_shaped_recap_yields_no_findings() {
    assert_eq!(validate_recap_contract(GOOD_RECAP), []);
}

#[test]
fn flags_a_missing_decisions_section() {
    let findings = validate_recap_contract("## Status\nall done, trust me");
    assert!(findings.contains(&ContractFinding::NoDecisionsSection));
}

#[test]
fn flags_a_lowercase_unverified_marker() {
    let findings = validate_recap_contract(
        "## Recent Decisions\nchose X because Y.\n(unverified: the Y figure)",
    );
    assert!(
        findings
            .iter()
            .any(|finding| matches!(finding, ContractFinding::NonstandardUnverified { .. }))
    );
}

#[test]
fn findings_never_throw() {
    drop(validate_recap_contract("free prose, no headers at all"));
}

#[test]
fn two_part_canonical_ref_resolves_the_seat_dir() {
    let tmp = Tmp::new();
    let seat = tmp.0.join("rigs/r1/seats/s1");
    fs::create_dir_all(&seat).expect("seat");
    fs::write(seat.join("RECAP.md"), "## Recent Decisions\nx").expect("recap");
    assert_eq!(
        resolve_authored_recap_pointer("s1@r1", &tmp.0),
        RecapPointer::Resolved {
            address: "seat:RECAP.md".to_owned(),
            chain_length: 0
        }
    );
}

#[test]
fn host_qualified_ref_uses_the_first_at_split() {
    let tmp = Tmp::new();
    let pointer = resolve_authored_recap_pointer("s1@r1@host-x", &tmp.0);
    let RecapPointer::Absent { reason } = pointer else {
        panic!("expected absence");
    };
    assert!(reason.contains(&format!(
        "{}",
        tmp.0.join("rigs/r1@host-x/seats/s1").display()
    )));
}

#[test]
fn a_malformed_ref_is_a_labeled_absence() {
    let tmp = Tmp::new();
    for bad in ["noatsign", "@r1", "s1@"] {
        let RecapPointer::Absent { reason } = resolve_authored_recap_pointer(bad, &tmp.0) else {
            panic!("expected absence");
        };
        assert!(reason.contains(bad));
    }
}

fn seed(dir: &std::path::Path) {
    write_seat_recap(dir, "## Decisions\nfirst recap", 1).expect("first");
    write_seat_recap(dir, "## Decisions\ncurrent recap", 2).expect("current");
}

fn chain_bodies(dir: &std::path::Path) -> Vec<String> {
    list_recap_chain(dir)
        .iter()
        .map(|path| body(path))
        .collect()
}

#[test]
fn archives_exclusively_when_links_fail() {
    for code in [95, 95, 1, 18] {
        let tmp = Tmp::new();
        seed(&tmp.0);
        set_recap_fault_for_tests(Some(RecapFault {
            phase: "",
            link_code: Some(code),
        }));
        write_seat_recap(&tmp.0, "## Decisions\nnew recap", 2).expect("publish");
        set_recap_fault_for_tests(None);
        assert_eq!(body(&tmp.0.join("RECAP.md")), "## Decisions\nnew recap");
        assert_eq!(
            chain_bodies(&tmp.0),
            ["## Decisions\nfirst recap", "## Decisions\ncurrent recap"]
        );
        assert_eq!(
            fs::read_dir(tmp.0.join("recap-superseded"))
                .expect("chain")
                .flatten()
                .count(),
            2
        );
        assert_eq!(names(&tmp.0), ["RECAP.md", "recap-superseded"]);
    }
}

#[test]
fn removes_an_owned_fallback_archive_after_failure() {
    for phase in ["fallback-copy", "publish"] {
        let tmp = Tmp::new();
        seed(&tmp.0);
        let before = chain_bodies(&tmp.0);
        set_recap_fault_for_tests(Some(RecapFault {
            phase,
            link_code: Some(95),
        }));
        assert!(write_seat_recap(&tmp.0, "## Decisions\nnew recap", 2).is_err());
        set_recap_fault_for_tests(None);
        assert_eq!(body(&tmp.0.join("RECAP.md")), "## Decisions\ncurrent recap");
        assert_eq!(chain_bodies(&tmp.0), before);
        assert_eq!(
            fs::read_dir(tmp.0.join("recap-superseded"))
                .expect("chain")
                .flatten()
                .count(),
            1
        );
        assert_eq!(names(&tmp.0), ["RECAP.md", "recap-superseded"]);
    }
}

#[test]
fn preserves_a_collided_archive_when_the_source_cannot_be_copied() {
    let tmp = Tmp::new();
    seed(&tmp.0);
    let before = chain_bodies(&tmp.0);
    set_recap_fault_for_tests(Some(RecapFault {
        phase: "source",
        link_code: None,
    }));
    let err = write_seat_recap(&tmp.0, "## Decisions\nnew recap", 2).expect_err("source");
    set_recap_fault_for_tests(None);
    assert!(err.message.contains("unreadable archive source"));
    assert_eq!(body(&tmp.0.join("RECAP.md")), "## Decisions\ncurrent recap");
    assert_eq!(chain_bodies(&tmp.0), before);
    assert_eq!(
        fs::read_dir(tmp.0.join("recap-superseded"))
            .expect("chain")
            .flatten()
            .count(),
        1
    );
    assert_eq!(names(&tmp.0), ["RECAP.md", "recap-superseded"]);
}

#[test]
fn preserves_readable_current_and_prior_chain_after_failure() {
    for phase in ["write", "archive", "link", "publish"] {
        let tmp = Tmp::new();
        seed(&tmp.0);
        let before = chain_bodies(&tmp.0);
        set_recap_fault_for_tests(Some(RecapFault {
            phase,
            link_code: None,
        }));
        assert!(write_seat_recap(&tmp.0, "## Decisions\nnew recap", 3).is_err());
        set_recap_fault_for_tests(None);
        assert_eq!(body(&tmp.0.join("RECAP.md")), "## Decisions\ncurrent recap");
        assert_eq!(chain_bodies(&tmp.0), before);
        assert_eq!(
            body(&list_recap_chain(&tmp.0)[0]),
            "## Decisions\nfirst recap"
        );
        assert_eq!(
            fs::read_dir(tmp.0.join("recap-superseded"))
                .expect("chain")
                .flatten()
                .count(),
            1
        );
        assert_eq!(names(&tmp.0), ["RECAP.md", "recap-superseded"]);
    }
}

#[test]
fn publishes_all_new_bytes_and_archives_the_previous_current() {
    let tmp = Tmp::new();
    seed(&tmp.0);
    write_seat_recap(&tmp.0, "## Decisions\nnew recap", 3).expect("new");
    assert_eq!(body(&tmp.0.join("RECAP.md")), "## Decisions\nnew recap");
    assert_eq!(
        chain_bodies(&tmp.0),
        ["## Decisions\nfirst recap", "## Decisions\ncurrent recap"]
    );
}
