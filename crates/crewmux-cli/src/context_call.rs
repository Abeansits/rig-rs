// Derived from openrig packages/cli/src/commands/context.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! HTTP helper for the daemon-backed `rig context` verbs.

use crewmux_api::Client;
use serde_json::Value;

use crate::autostart::ensure_daemon;
use crate::error::Error;

pub(crate) fn push_query(path: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        path.push('&');
        path.push_str(key);
        path.push('=');
        path.push_str(&escape(value));
    }
}

pub(crate) fn call(method: &str, path: &str, body: Option<&[u8]>) -> Result<(u16, Value), Error> {
    let client: Client = ensure_daemon()?;
    let response = client
        .request(method, path, body)
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    let value = serde_json::from_str(&response.body).unwrap_or(Value::Null);
    Ok((response.status, value))
}

pub(crate) fn expect(status: u16, want: u16, value: &Value) -> Result<(), Error> {
    if status == want {
        Ok(())
    } else {
        fail(status, value)
    }
}

pub(crate) fn fail<T>(status: u16, value: &Value) -> Result<T, Error> {
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .map_or_else(|| format!("Daemon returned HTTP {status}"), str::to_owned);
    Err(Error::User(message))
}

pub(crate) fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

pub(crate) fn escape(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                if let Some(ch) = char::from_u32(u32::from(byte)) {
                    out.push(ch);
                }
            }
            _ => {
                const HEX: &str = "0123456789ABCDEF";
                out.push('%');
                if let Some(ch) = HEX.chars().nth(usize::from(byte / 16)) {
                    out.push(ch);
                }
                if let Some(ch) = HEX.chars().nth(usize::from(byte % 16)) {
                    out.push(ch);
                }
            }
        }
    }
    out
}
