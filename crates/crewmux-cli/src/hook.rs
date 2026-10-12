//! `rig hook <event>`. Always exits 0. A down daemon appends the spool.

use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crewmux_api::{Client, daemon_socket, rig_home, seat_hook_path};
use serde_json::{Value, json};

use crate::output;

pub(crate) fn run(event: &str) -> i32 {
    let mut stdin = String::new();
    if io::stdin().read_to_string(&mut stdin).is_err() {
        output::stderr_line("hook json: failed to read stdin");
        return 0;
    }
    let Ok(payload) = payload(&stdin, event) else {
        output::stderr_line("hook json: malformed stdin");
        return 0;
    };
    let run_id = std::env::var("RIG_RUN_ID")
        .ok()
        .filter(|text| !text.is_empty());
    let body = json!({"run_id": run_id, "payload": payload});
    let Ok(bytes) = serde_json::to_vec(&body) else {
        output::stderr_line("hook json: failed to encode");
        return 0;
    };
    let home = std::env::var_os("RIG_HOME").map_or_else(rig_home, PathBuf::from);
    let seat = std::env::var("RIG_SEAT_KEY").unwrap_or_default();
    let path = seat_hook_path(&seat);
    let client = Client::over_uds(daemon_socket(&home)).with_timeout(Duration::from_millis(800));
    if client.request("POST", &path, Some(&bytes)).is_err() {
        output::stderr_line("hook: daemon unreachable; spooling");
        if let Err(err) = spool(&home, &seat, &body) {
            output::stderr_line(&err);
        }
    }
    0
}

fn payload(stdin: &str, event: &str) -> Result<Value, ()> {
    let mut value: Value = serde_json::from_str(stdin).map_err(|_| ())?;
    let Some(object) = value.as_object_mut() else {
        return Err(());
    };
    let missing = object
        .get("hook_event_name")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty);
    if missing {
        object.insert(
            "hook_event_name".to_owned(),
            Value::String(event.to_owned()),
        );
    }
    Ok(value)
}

fn spool(home: &Path, seat: &str, body: &Value) -> Result<(), String> {
    if !safe_component(seat) {
        return Err("hook spool: missing seat key".to_owned());
    }
    let path = home.join("seats").join(seat).join("hooks.spool.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| err.to_string())?;
    let mode = std::fs::Permissions::from_mode(0o600);
    file.set_permissions(mode).map_err(|err| err.to_string())?;
    let line = serde_json::to_string(body).map_err(|err| err.to_string())?;
    writeln!(file, "{line}").map_err(|err| err.to_string())
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('\0')
}
