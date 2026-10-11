//! CSV reader and writer for `parity/ledger.csv`.

use super::types::{COLUMNS, LedgerRow, LedgerStatus, optional, parse_pe};

/// Parse the ledger. Structural problems and unknown statuses are errors in the list.
///
/// # Errors
///
/// Returns the human-readable problems. An empty list is not used; success is `Ok`.
pub fn parse_csv(text: &str) -> Result<Vec<LedgerRow>, Vec<String>> {
    let mut errors = Vec::new();
    let mut rows = Vec::new();
    let mut lines = text.lines().filter(|line| !line.is_empty());
    let Some(header) = lines.next() else {
        return Err(vec!["ledger is empty".to_owned()]);
    };
    match parse_record(header) {
        Ok(fields) if fields == COLUMNS.map(str::to_owned).to_vec() => {}
        Ok(fields) => {
            errors.push(format!(
                "ledger header is {fields:?}, expected ts_path,slice,status,rust_test,pe_id,note"
            ));
        }
        Err(error) => errors.push(error),
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for (index, line) in lines.enumerate() {
        let line_no = index.saturating_add(2);
        let fields = match parse_record(line) {
            Ok(fields) => fields,
            Err(error) => {
                errors.push(format!("line {line_no}: {error}"));
                continue;
            }
        };
        if fields.len() != COLUMNS.len() {
            errors.push(format!(
                "line {line_no}: expected 6 columns, found {}",
                fields.len()
            ));
            continue;
        }
        let status_text = field(&fields, 2);
        let Ok(status) = LedgerStatus::parse(status_text) else {
            errors.push(format!("line {line_no}: unknown status {status_text}"));
            continue;
        };
        let pe_id = match parse_pe(field(&fields, 4)) {
            Ok(pe) => pe,
            Err(error) => {
                errors.push(format!("line {line_no}: {error}"));
                continue;
            }
        };
        rows.push(LedgerRow {
            ts_path: field(&fields, 0).to_owned(),
            slice: optional(field(&fields, 1)),
            status,
            rust_test: optional(field(&fields, 3)),
            pe_id,
            note: field(&fields, 5).to_owned(),
        });
    }
    if errors.is_empty() {
        Ok(rows)
    } else {
        Err(errors)
    }
}

/// Quote every field so commas in notes stay in one column.
#[must_use]
pub fn to_csv(rows: &[LedgerRow]) -> String {
    let mut out = String::new();
    out.push_str(&COLUMNS.map(quote).join(","));
    out.push('\n');
    for row in rows {
        let fields = [
            quote(&row.ts_path),
            quote(row.slice.as_deref().unwrap_or("")),
            quote(row.status.as_str()),
            quote(row.rust_test.as_deref().unwrap_or("")),
            quote(row.pe_id.as_ref().map_or("", |pe| pe.as_str())),
            quote(&row.note),
        ];
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

fn field(fields: &[String], index: usize) -> &str {
    fields.get(index).map_or("", String::as_str)
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

fn parse_record(line: &str) -> Result<Vec<String>, String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(ch);
            }
        } else if ch == '"' && current.is_empty() {
            in_quotes = true;
        } else if ch == ',' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    if in_quotes {
        return Err("unterminated quote".to_owned());
    }
    fields.push(current);
    Ok(fields)
}
