//! Out-of-scope path rules, taken from the porting map.
//!
//! First match wins. The patterns are narrower than a raw substring search for
//! `discover`, `compose`, or `proof`, so in-scope tests that merely share those
//! words stay `todo`.

use regex::Regex;

use crate::error::Error;

struct Rule {
    pattern: Regex,
    note: &'static str,
}

/// Waiver note for `ts_path`, or `None` when the file stays `todo`.
///
/// # Errors
///
/// Returns [`Error::Pattern`] if a rule expression cannot be compiled.
pub fn waiver_for(ts_path: &str) -> Result<Option<&'static str>, Error> {
    for rule in rules()? {
        if rule.pattern.is_match(ts_path) {
            return Ok(Some(rule.note));
        }
    }
    Ok(None)
}

fn rules() -> Result<Vec<Rule>, Error> {
    let specs = [
        (
            r"gateway|slack",
            "Slack/human gateway is out of scope (porting map)",
        ),
        (
            r"bundle",
            "Bundles and publishing are out of scope (porting map)",
        ),
        (r"cross-host", "Cross-host is out of scope (porting map)"),
        (r"cmux", "cmux is out of scope (porting map)"),
        (
            r"(^|/)(pi|omp)-|capture-pi\.test\.tsx?$",
            "Pi and Oh My Pi are out of scope (porting map)",
        ),
        (
            r"web-ui|(^|/)ui/",
            "Legacy web UI is out of scope (porting map)",
        ),
        (
            r"plugin|agent-image",
            "Plugins and agent images are out of scope (porting map)",
        ),
        (
            r"docker|service-secret",
            "Docker service rigs and secrets are out of scope (porting map)",
        ),
        (
            r"(^|/)adopt\.test\.tsx?$|tmux-discovery|discovery-coordinator|discovery-routes",
            "Discover/adopt of existing sessions is out of scope (porting map)",
        ),
        (
            r"(^|/)migration-|migrations-",
            "Migration helpers are out of scope (porting map)",
        ),
        (
            r"(^|/)proof-|(^|/)demo-|openrig-compat",
            "Legacy compat shims, demos, and proof commands are out of scope (porting map)",
        ),
    ];
    let mut out = Vec::new();
    for (pattern, note) in specs {
        out.push(Rule {
            pattern: Regex::new(pattern).map_err(|err| Error::Pattern(err.to_string()))?,
            note,
        });
    }
    Ok(out)
}
