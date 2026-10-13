//! Pack-ref and version tokens.
//!
//! Derived from openrig packages/daemon/src/domain/context-packs/ref-safety.ts
//! (v0.6.9, Apache-2.0); ported to Rust and modified.

/// One path segment: leading alnum, then alnum / `.` / `_` / `-`, at most 64 chars.
#[must_use]
pub fn is_safe_segment(segment: &str) -> bool {
    bounded_token(segment, 64, false)
}

/// A pack version is one delimiter-free token of at most 32 characters.
#[must_use]
pub fn is_safe_version(version: &str) -> bool {
    bounded_token(version, 32, true)
}

/// A path-like ref is one or more `/`-separated safe segments.
#[must_use]
pub fn is_safe_pack_ref(refer: &str) -> bool {
    if refer.is_empty() {
        return false;
    }
    refer.split('/').all(is_safe_segment)
}

/// Throwing form of [`is_safe_pack_ref`].
///
/// # Errors
///
/// Returns the TypeScript sentence when `refer` is not a safe pack ref.
pub fn assert_safe_pack_ref(refer: &str) -> Result<(), String> {
    if is_safe_pack_ref(refer) {
        Ok(())
    } else {
        Err(format!(
            "unsafe pack ref '{refer}' — a ref must be one or more '/'-separated segments, each matching \
             [A-Za-z0-9][A-Za-z0-9._-]{{0,63}} (no '.'/'..' , no absolute path, no empty segment, no \
             whitespace or injection), so packs stay inside the context store root."
        ))
    }
}

fn bounded_token(token: &str, max: usize, plus: bool) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let mut len = 1usize;
    for ch in chars {
        len = len.saturating_add(1);
        if len > max || !token_char(ch, plus) {
            return false;
        }
    }
    true
}

fn token_char(ch: char, plus: bool) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') || (plus && ch == '+')
}

#[cfg(test)]
mod tests {
    use super::{assert_safe_pack_ref, is_safe_pack_ref, is_safe_version};

    #[test]
    fn accepts_path_like_refs() {
        assert!(is_safe_pack_ref("packs/compaction-restore"));
        assert!(is_safe_pack_ref("compaction-restore"));
        assert!(is_safe_pack_ref("a/b/c.d_e-f"));
        assert!(assert_safe_pack_ref("packs/compaction-restore").is_ok());
    }

    #[test]
    fn rejects_unsafe_refs() {
        let long = "x".repeat(65);
        let bad = [
            "../evil",
            "packs/../evil",
            "packs/..",
            "/abs/path",
            "packs//nested",
            "packs/",
            "",
            ".",
            "packs/.hidden",
            "packs/na me",
            "packs/na:me",
            "packs/na\nme",
            "packs/na\tme",
            long.as_str(),
        ];
        for refer in bad {
            assert!(!is_safe_pack_ref(refer), "{refer:?}");
            assert!(assert_safe_pack_ref(refer).is_err());
        }
    }

    #[test]
    fn version_token() {
        assert!(is_safe_version("1.0.0"));
        assert!(is_safe_version("2026-08-04"));
        assert!(is_safe_version("v1_2+build"));
        assert!(!is_safe_version(&"x".repeat(300)));
        assert!(!is_safe_version("1.0 0"));
        assert!(!is_safe_version("1:0:0"));
        assert!(!is_safe_version("1/0"));
        assert!(!is_safe_version("@1.0"));
        assert!(!is_safe_version(""));
    }
}
