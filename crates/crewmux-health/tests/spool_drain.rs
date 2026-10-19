//! Spool lines drain in order. A trailing partial line is left behind.

use crewmux_health::compaction::{parse_spool_line, split_spool_text};

#[test]
fn spool_drain() {
    let text = concat!(
        "{\"run_id\":\"a\",\"payload\":{\"hook_event_name\":\"Stop\",\"session_id\":\"abc\"}}\n",
        "{\"run_id\":\"b\",\"payload\":{\"hook_event_name\":\"SessionEnd\",\"session_id\":\"abc\",\"reason\":\"other\"}}\n",
        "{\"partial\"",
    );
    let split = split_spool_text(text);
    assert_eq!(split.complete.len(), 2);
    assert_eq!(split.remainder, "{\"partial\"");
    let first = parse_spool_line(&split.complete[0]).expect("first");
    let second = parse_spool_line(&split.complete[1]).expect("second");
    assert_eq!(first.run_id.as_deref(), Some("a"));
    assert_eq!(second.run_id.as_deref(), Some("b"));
    assert_eq!(first.payload["hook_event_name"], "Stop");
    assert_eq!(second.payload["hook_event_name"], "SessionEnd");
    assert!(parse_spool_line(&split.remainder).is_none());
}
