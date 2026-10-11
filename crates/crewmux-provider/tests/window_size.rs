//! Effective global `window-size manual` lines. Comments and quotes count.

use crewmux_provider::global_window_size_manual;

#[test]
fn flag_group_containing_g_is_global() {
    assert!(global_window_size_manual("set -g window-size manual\n"));
    assert!(global_window_size_manual("set -wg window-size manual\n"));
    assert!(global_window_size_manual(
        "set-option -gw window-size manual\n"
    ));
    assert!(global_window_size_manual("setw -g window-size manual\n"));
    assert!(global_window_size_manual(
        "set-window-option -ag window-size manual\n"
    ));
    assert!(!global_window_size_manual("set -w window-size manual\n"));
}

#[test]
fn quoted_manual_value_matches() {
    assert!(global_window_size_manual("set -g window-size \"manual\"\n"));
    assert!(global_window_size_manual("set -g window-size 'manual'\n"));
    assert!(!global_window_size_manual(
        "set -g window-size \"latest\"\n"
    ));
}

#[test]
fn comment_is_not_a_setting() {
    assert!(!global_window_size_manual("# set -g window-size manual\n"));
    assert!(global_window_size_manual(
        "set -g window-size manual # kept\n"
    ));
}
