//! Role names in YAML source order.
//!
//! `serde_json` sorts object keys. The skill-ref advisory names the first role
//! in the file, so the parser records that order from the text.

/// Role keys directly under `roles:`, in the order they appear.
#[must_use]
pub(crate) fn role_order(text: &str) -> Vec<String> {
    let mut order = Vec::new();
    let mut roles_at: Option<usize> = None;
    for line in text.lines() {
        let trimmed = strip_comment(line).trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.chars().take_while(|ch| *ch == ' ').count();
        if roles_at.is_none() {
            if trimmed == "roles:" {
                roles_at = Some(indent);
            }
            continue;
        }
        let Some(base) = roles_at else {
            continue;
        };
        if indent <= base {
            break;
        }
        if indent == base.saturating_add(2)
            && let Some(name) = trimmed.strip_suffix(':')
            && !name.is_empty()
            && !name.contains([' ', '"', '\''])
        {
            order.push(name.to_owned());
        }
    }
    order
}

fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (index, ch) in line.char_indices() {
        if let Some(open) = quote {
            if ch == open {
                quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        if ch == '#' {
            return line.get(..index).unwrap_or(line);
        }
    }
    line
}
