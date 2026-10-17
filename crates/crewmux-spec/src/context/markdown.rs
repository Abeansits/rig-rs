//! Addressable markdown: one grammar, `name#H2-slug/H3-slug`.
//!
//! Derived from openrig packages/daemon/src/domain/markdown-address.ts
//! (v0.6.9, Apache-2.0); ported to Rust and modified.

/// Address parse or resolution failure.
#[derive(Debug, PartialEq, Eq)]
pub struct AddressError {
    /// TypeScript sentence.
    pub message: String,
}

impl AddressError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A parsed address. The caller owns the pre-`#` ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedAddress {
    /// Library ref or tree path.
    pub refer: String,
    /// Zero, one, or two header slugs.
    pub header_path: Vec<String>,
}

/// One H2 or H3 section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownSection {
    /// Header level, 2 or 3.
    pub level: u8,
    /// Raw title.
    pub title: String,
    /// `[h2]` or `[h2, h3]`.
    pub header_path: Vec<String>,
    /// Zero-based header line.
    pub header_line: usize,
    /// Full span through the line before the next same-or-higher header.
    pub text: String,
    /// Own text through the line before the next header of any level.
    pub own_text: String,
}

/// Addressability findings. The caller decides the gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressFinding {
    /// Two sections share a path.
    Duplicate {
        /// Joined path.
        header_path: String,
        /// Header lines.
        lines: Vec<usize>,
    },
    /// A named header slugifies to an empty segment.
    Unaddressable {
        /// Joined path.
        header_path: String,
        /// Header line.
        line: usize,
        /// Raw title.
        title: String,
    },
    /// A fence was still open at EOF.
    UnterminatedFence {
        /// Opening line.
        line: usize,
    },
}

/// Lowercase, strip emphasis markers, collapse non-alphanumerics to one hyphen.
#[must_use]
pub fn slugify_header(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut out = String::new();
    let mut hyphen = false;
    let mut started = false;
    for ch in lower.chars() {
        if matches!(ch, '`' | '*' | '_' | '~') {
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            if hyphen && started {
                out.push('-');
            }
            hyphen = false;
            started = true;
            out.push(ch);
        } else if started {
            hyphen = true;
        }
    }
    out
}

/// Parse `ref`, `ref#h2`, or `ref#h2/h3`.
///
/// # Errors
///
/// Returns [`AddressError`] when the grammar is not that one form.
pub fn parse_address(address: &str) -> Result<ParsedAddress, AddressError> {
    let hashes = address.chars().filter(|ch| *ch == '#').count();
    if hashes > 1 {
        return Err(AddressError::new(format!(
            "address '{address}' has {hashes} '#' separators — the one form is name#H2-slug/H3-slug (a single '#')."
        )));
    }
    let (refer, header) = match address.split_once('#') {
        Some((refer, header)) => (refer, Some(header)),
        None => (address, None),
    };
    if refer.is_empty() {
        return Err(AddressError::new(format!(
            "address '{address}' has an empty ref before '#' — an address always names its file/ref."
        )));
    }
    let Some(header) = header else {
        return Ok(ParsedAddress {
            refer: refer.to_owned(),
            header_path: Vec::new(),
        });
    };
    let header_path: Vec<String> = header.split('/').map(str::to_owned).collect();
    if header_path.iter().any(String::is_empty) {
        return Err(AddressError::new(format!(
            "address '{address}' has an empty header segment — the form is name#H2-slug or name#H2-slug/H3-slug."
        )));
    }
    if header_path.len() > 2 {
        return Err(AddressError::new(format!(
            "address '{address}' goes {} levels deep — addresses target H2 and H3 only (Q1 ruling), so the deepest form is name#H2-slug/H3-slug.",
            header_path.len()
        )));
    }
    Ok(ParsedAddress {
        refer: refer.to_owned(),
        header_path,
    })
}

struct Hit {
    level: u8,
    title: String,
    line: usize,
}

struct Scan {
    hits: Vec<Hit>,
    fence_line: Option<usize>,
}

fn scan_headers(lines: &[&str]) -> Scan {
    let mut hits = Vec::new();
    let mut fence: Option<(char, usize, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some((marker, length, rest)) = fence_open(line) {
            match fence {
                None if marker == '`' && rest.contains('`') => {}
                None => fence = Some((marker, length, index)),
                Some((open, need, _)) if open == marker && length >= need && rest_blank(rest) => {
                    fence = None;
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if let Some(hit) = header_hit(line, index) {
            hits.push(hit);
        }
    }
    Scan {
        fence_line: fence.map(|(_, _, line)| line),
        hits,
    }
}

fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let rest = line.trim_start_matches(' ');
    let indent = line.len().saturating_sub(rest.len());
    if indent > 3 {
        return None;
    }
    let marker = rest.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = rest.chars().take_while(|ch| *ch == marker).count();
    if length < 3 {
        return None;
    }
    let after = rest.get(length..).unwrap_or("");
    Some((marker, length, after))
}

fn rest_blank(rest: &str) -> bool {
    rest.chars().all(|ch| ch == ' ' || ch == '\t' || ch == '\r')
}

fn header_hit(line: &str, index: usize) -> Option<Hit> {
    let rest = line.trim_start_matches(' ');
    let indent = line.len().saturating_sub(rest.len());
    if indent > 3 || !rest.starts_with('#') {
        return None;
    }
    let level = rest.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let after = rest.get(level..).unwrap_or("");
    if !after.is_empty() && !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let title = after.trim().to_owned();
    if !after.trim().is_empty() && after.trim() != title.trim() {
        return None;
    }
    let level = u8::try_from(level).unwrap_or(0);
    Some(Hit {
        level,
        title,
        line: index,
    })
}

/// Parse H2/H3 sections. A matched span always includes its header line.
#[must_use]
pub fn parse_sections(text: &str) -> Vec<MarkdownSection> {
    let owned: Vec<&str> = text.split('\n').collect();
    let scan = scan_headers(&owned);
    let mut sections = Vec::new();
    let mut current_h2: Option<String> = None;
    for (idx, hit) in scan.hits.iter().enumerate() {
        if hit.level < 2 || hit.level > 3 {
            if hit.level < 2 {
                current_h2 = if hit.title.is_empty() {
                    Some(String::new())
                } else {
                    None
                };
            }
            continue;
        }
        let slug = slugify_header(&hit.title);
        if hit.level == 2 {
            current_h2 = Some(slug.clone());
        }
        let header_path = if hit.level == 2 {
            vec![slug]
        } else if let Some(parent) = &current_h2 {
            vec![parent.clone(), slug]
        } else {
            vec![slug]
        };
        let full_end = scan
            .hits
            .iter()
            .skip(idx.saturating_add(1))
            .find(|next| next.level <= hit.level)
            .map_or(owned.len(), |next| next.line);
        let own_end = scan
            .hits
            .get(idx.saturating_add(1))
            .map_or(owned.len(), |next| next.line);
        sections.push(MarkdownSection {
            level: hit.level,
            title: hit.title.clone(),
            header_path,
            header_line: hit.line,
            text: owned.get(hit.line..full_end).unwrap_or(&[]).join("\n"),
            own_text: owned.get(hit.line..own_end).unwrap_or(&[]).join("\n"),
        });
    }
    sections
}

/// Resolve a header path. A miss names the candidates.
///
/// # Errors
///
/// Returns [`AddressError`] when the path is empty, ambiguous, or missing.
pub fn resolve_address(
    text: &str,
    header_path: &[String],
) -> Result<MarkdownSection, AddressError> {
    if header_path.is_empty() {
        return Err(AddressError::new(
            "resolveAddress needs at least one header slug; a bare ref resolves to the whole file at the caller.",
        ));
    }
    let sections = parse_sections(text);
    let wanted = header_path.join("/");
    let hits: Vec<&MarkdownSection> = sections
        .iter()
        .filter(|section| section.header_path.join("/") == wanted)
        .collect();
    if hits.len() > 1 {
        let lines = hits
            .iter()
            .map(|hit| hit.header_line.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AddressError::new(format!(
            "address '#{wanted}' is AMBIGUOUS in this file — {} sections share the path (header lines {lines}). Fix the duplicate headers; serving any one of them would be a silent wrong answer.",
            hits.len()
        )));
    }
    if let Some(hit) = hits.first() {
        return Ok((*hit).clone());
    }
    let parent = header_path
        .get(..header_path.len().saturating_sub(1))
        .unwrap_or(&[])
        .join("/");
    let candidates: Vec<String> = sections
        .iter()
        .filter(|section| {
            let prefix = section
                .header_path
                .get(..section.header_path.len().saturating_sub(1))
                .unwrap_or(&[]);
            prefix.join("/") == parent
        })
        .map(|section| section.header_path.join("/"))
        .collect();
    let hint = if candidates.is_empty() {
        let all = sections
            .iter()
            .map(|section| section.header_path.join("/"))
            .collect::<Vec<_>>()
            .join(", ");
        let shown = if all.is_empty() {
            "(none)"
        } else {
            all.as_str()
        };
        format!(
            "The file has {} addressable section(s): {shown}.",
            sections.len()
        )
    } else {
        let label = if parent.is_empty() {
            "(top)"
        } else {
            parent.as_str()
        };
        format!(
            "Addressable sections under '{label}': {}.",
            candidates.join(", ")
        )
    };
    Err(AddressError::new(format!(
        "address '#{wanted}' matches no header in this file — composition stops here rather than thinning the walk. {hint}"
    )))
}

/// Compose-gate findings. Does not throw.
#[must_use]
pub fn validate_addressability(text: &str) -> Vec<AddressFinding> {
    let owned: Vec<&str> = text.split('\n').collect();
    let scan = scan_headers(&owned);
    let sections = parse_sections(text);
    let mut findings = Vec::new();
    if let Some(line) = scan.fence_line {
        findings.push(AddressFinding::UnterminatedFence { line });
    }
    let mut seen: Vec<(String, Vec<usize>)> = Vec::new();
    let mut blank_parent = false;
    let mut header_index = 0usize;
    for section in &sections {
        while header_index < scan.hits.len()
            && scan
                .hits
                .get(header_index)
                .is_some_and(|hit| hit.line <= section.header_line)
        {
            if let Some(header) = scan.hits.get(header_index)
                && header.level <= 2
            {
                blank_parent = header.title.is_empty();
            }
            header_index = header_index.saturating_add(1);
        }
        if skip_blank(section, blank_parent) {
            continue;
        }
        if section.header_path.iter().any(String::is_empty) {
            findings.push(AddressFinding::Unaddressable {
                header_path: section.header_path.join("/"),
                line: section.header_line,
                title: section.title.clone(),
            });
            continue;
        }
        let key = section.header_path.join("/");
        if let Some((_, lines)) = seen.iter_mut().find(|(path, _)| path == &key) {
            lines.push(section.header_line);
        } else {
            seen.push((key, vec![section.header_line]));
        }
    }
    for (header_path, lines) in seen {
        if lines.len() > 1 {
            findings.push(AddressFinding::Duplicate { header_path, lines });
        }
    }
    findings
}

fn skip_blank(section: &MarkdownSection, blank_parent: bool) -> bool {
    if section.title.is_empty() {
        return true;
    }
    blank_parent
        && section.header_path.len() == 2
        && section
            .header_path
            .get(1)
            .is_some_and(|segment| !segment.is_empty())
}
