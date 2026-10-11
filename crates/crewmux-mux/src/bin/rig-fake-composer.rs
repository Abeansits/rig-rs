//! Bracketed-paste log and burst producer used by the tmux integration tests.
//!
//! `composer` prints `READY`, then `P <len>\n<bytes>\n` for each bracketed paste
//! and `S\n` for a bare CR or LF (a submit). `produce <path>` writes one
//! deterministic burst to `path` and to stdout.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

const START: &[u8] = b"\x1b[200~";
const END: &[u8] = b"\x1b[201~";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            write_err(&err);
            ExitCode::from(1)
        }
    }
}

fn write_err(err: &io::Error) {
    #[expect(
        clippy::print_stderr,
        reason = "the composer reports failures on stderr"
    )]
    {
        eprintln!("rig-fake-composer: {err}");
    }
}

fn run() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("composer") => composer(),
        Some("produce") => {
            let path = args
                .next()
                .ok_or_else(|| io::Error::other("produce needs an output path"))?;
            produce(Path::new(&path))
        }
        _ => Err(io::Error::other(
            "usage: rig-fake-composer composer | produce <path>",
        )),
    }
}

fn composer() -> io::Result<()> {
    let mut input = io::stdin();
    let mut output = io::stdout();
    // tmux only brackets `paste-buffer -p` after the pane asks for it.
    output.write_all(b"\x1b[?2004h")?;
    writeln!(output, "READY")?;
    output.flush()?;
    let mut pending = Vec::new();
    let mut buf = [0_u8; 8192];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(chunk) = buf.get(..n) {
            pending.extend_from_slice(chunk);
        }
        drain_composer(&mut pending, &mut output)?;
    }
    Ok(())
}

fn drain_composer(pending: &mut Vec<u8>, output: &mut impl Write) -> io::Result<()> {
    let mut cursor = 0usize;
    while cursor < pending.len() {
        let rest = pending.get(cursor..).unwrap_or_default();
        if rest.starts_with(START) {
            if let Some(end) = find_at(rest, END) {
                let data = rest.get(START.len()..end).unwrap_or_default();
                write_paste(output, data)?;
                cursor += end + END.len();
                continue;
            }
            break;
        }
        if is_start_prefix(rest) {
            break;
        }
        if rest.first() == Some(&b'\r') || rest.first() == Some(&b'\n') {
            writeln!(output, "S")?;
            output.flush()?;
            cursor += 1;
            continue;
        }
        cursor += 1;
    }
    if cursor > 0 {
        pending.drain(..cursor);
    }
    Ok(())
}

fn write_paste(output: &mut impl Write, data: &[u8]) -> io::Result<()> {
    writeln!(output, "P {}", data.len())?;
    output.write_all(data)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn is_start_prefix(buf: &[u8]) -> bool {
    buf.len() < START.len() && START.starts_with(buf)
}

fn find_at(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    for index in 0..=last {
        if hay.get(index..index + needle.len()) == Some(needle) {
            return Some(index);
        }
    }
    None
}

fn produce(path: &Path) -> io::Result<()> {
    let bytes = burst();
    std::fs::write(path, &bytes)?;
    let mut output = io::stdout();
    output.write_all(&bytes)?;
    output.flush()
}

fn burst() -> Vec<u8> {
    let pattern = b"ASCII \x1b[31mred\x1b[0m \xc2\xa9\r\n";
    let mut out = Vec::with_capacity(400_000);
    while out.len() < 400_000 {
        out.extend_from_slice(pattern);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::drain_composer;

    #[test]
    fn bracketed_paste_is_framed_and_cr_submits() {
        let mut pending = b"\x1b[200~one\ntwo\x1b[201~\r".to_vec();
        let mut out = Vec::new();
        drain_composer(&mut pending, &mut out).expect("drain");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.starts_with("P 7\n"), "{text}");
        assert!(text.contains("one\ntwo"));
        assert!(text.ends_with("S\n"), "{text}");
        assert_eq!(pending, Vec::<u8>::new());
    }
}
