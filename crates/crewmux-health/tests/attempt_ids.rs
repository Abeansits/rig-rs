//! Managed attempt numbers are per seat and start at 1.

use crewmux_health::compaction::{AttemptId, Book};

#[test]
fn manual_attempt_numbers_start_at_one_for_each_seat() {
    let mut book = Book::new();
    let first = book.next_manual_id("seat-a");
    let other = book.next_manual_id("seat-b");
    assert_eq!(first.as_str(), "manual-seat-a-1");
    assert_eq!(other.as_str(), "manual-seat-b-1");
    book.fail_unsent(
        "seat-a",
        AttemptId(first.as_str().to_owned()),
        "send_failed",
        0,
    );
    let again = book.next_manual_id("seat-a");
    assert_eq!(again.as_str(), "manual-seat-a-2");
    let stored = book.attempts().to_vec();
    let mut restored = Book::new();
    restored.load(stored);
    assert_eq!(
        restored.next_manual_id("seat-a").as_str(),
        "manual-seat-a-2"
    );
    assert_eq!(
        restored.next_manual_id("seat-b").as_str(),
        "manual-seat-b-1"
    );
}
