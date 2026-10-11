//! Parser properties: valid seat names parse, and unknown keys do not.

use crewmux_spec::parse;
use proptest::prelude::*;

proptest! {
    #[test]
    fn alphabetic_seat_names_parse(name in "[a-z]{1,12}") {
        let text = format!("name: rig\nseats:\n  - name: {name}\n    runtime: stub\n");
        let spec = parse(&text).expect("spec");
        let seat = spec.seats.first().expect("seat");
        prop_assert_eq!(seat.name.as_str(), name.as_str());
        prop_assert!(seat.cwd.is_none());
    }
}

proptest! {
    #[test]
    fn unknown_top_level_keys_are_rejected(key in "[a-z]{3,8}") {
        prop_assume!(key != "name" && key != "seats");
        let text = format!("name: rig\n{key}: 1\nseats:\n  - name: one\n    runtime: stub\n");
        let err = parse(&text).expect_err("unknown");
        prop_assert!(err.line >= 1);
        prop_assert!(err.col >= 1);
        prop_assert!(err.message.contains("unknown field") || err.path == key);
    }
}
