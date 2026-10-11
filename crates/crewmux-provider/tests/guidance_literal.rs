//! Managed guidance blocks keep `$` sequences literal. Markers match the file format.

use crewmux_provider::{MemoryFs, merge_managed_block};

const DOLLAR_TEXT: &str = "echo $$ pid; printf $'\\n'; regex $& and $` and $'; keep $1 literal";
const FILE: &str = "/ws/AGENTS.md";

fn managed(id: &str, content: &str) -> String {
    format!(
        "<!-- BEGIN OpenRig MANAGED BLOCK: {id} -->\n{content}\n<!-- END OpenRig MANAGED BLOCK: {id} -->"
    )
}

#[test]
fn dollar_sequences_stay_literal() {
    let mut fs = MemoryFs::default();
    merge_managed_block(&mut fs, FILE, "core", DOLLAR_TEXT);
    assert_eq!(fs.files[FILE], managed("core", DOLLAR_TEXT));
    for _ in 0..3 {
        merge_managed_block(&mut fs, FILE, "core", DOLLAR_TEXT);
        assert_eq!(fs.files[FILE].trim_end(), managed("core", DOLLAR_TEXT));
    }

    let before = "User intro with $& and $$\n\n";
    let after = "\n\nUser notes with $' and $`\n";
    let mut fs = MemoryFs {
        files: [(
            FILE.to_owned(),
            format!("{before}{}{after}", managed("core", "old guidance")),
        )]
        .into_iter()
        .collect(),
    };
    let expected = format!("{before}{}{after}", managed("core", DOLLAR_TEXT));
    for _ in 0..3 {
        merge_managed_block(&mut fs, FILE, "core", DOLLAR_TEXT);
        assert_eq!(fs.files[FILE].trim_end(), expected.trim_end());
    }

    let legacy = "<!-- BEGIN RIGGED MANAGED BLOCK: core -->\nold guidance\n<!-- END RIGGED MANAGED BLOCK: core -->";
    let before = "Before $$ text\n\n";
    let after = "\n\nAfter $' text\n";
    let mut fs = MemoryFs {
        files: [(FILE.to_owned(), format!("{before}{legacy}{after}"))]
            .into_iter()
            .collect(),
    };
    merge_managed_block(&mut fs, FILE, "core", DOLLAR_TEXT);
    let expected = format!("{before}{}{after}", managed("core", DOLLAR_TEXT));
    assert_eq!(fs.files[FILE].trim_end(), expected.trim_end());
    assert!(!fs.files[FILE].contains("RIGGED MANAGED BLOCK"));
}
