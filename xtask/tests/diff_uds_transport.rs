//! AC3: GET /healthz over the rig-rs Unix socket, and TCP rejected at load time.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use crewmux_api::Client;
use xtask::Error;
use xtask::diff::{Observation, Project, Side, Transport, bind_transport, load_case, project};

#[test]
fn diff_uds_transport() {
    let rig_home = tempfile::TempDir::with_prefix("rigrs-uds-").unwrap();
    fs::create_dir_all(rig_home.path().join("run")).unwrap();
    let sock = crewmux_api::daemon_socket(rig_home.path());
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || serve_healthz(&listener));
    let response = Client::over_uds(&sock).get_healthz().unwrap();
    server.join().unwrap().unwrap();
    assert_eq!(response.status, 200);
    assert!(
        response.body.contains("\"status\":\"ok\""),
        "{}",
        response.body
    );

    let low = health("{\"status\":\"ok\",\"pid\":1}");
    let high = health("{\"status\":\"ok\",\"pid\":2}");
    assert_eq!(
        project(low, Project::HealthShape).stdout,
        project(high, Project::HealthShape).stdout
    );

    let dir = tempfile::TempDir::with_prefix("rigrs-tcp-").unwrap();
    let markdown = dir.path().join("parity-exceptions.md");
    fs::write(&markdown, "## PE-001\n").unwrap();
    let case_path = dir.path().join("tcp.json");
    fs::write(
        &case_path,
        r#"{"id":"tcp-rejected","rs_transport":"tcp","steps":[{"op":"http","method":"GET","path":"/healthz"}]}"#,
    )
    .unwrap();
    let error = load_case(&case_path, &markdown).unwrap_err();
    assert!(matches!(error, Error::RsTcpRejected { .. }), "{error}");
    let side = Side::Rs {
        bin: PathBuf::from("rig"),
    };
    let addr = "127.0.0.1:9".parse().unwrap();
    let rejected = bind_transport(&side, &Transport::Tcp(addr)).unwrap_err();
    assert!(matches!(rejected, Error::RsTcpRejected { .. }));
}

fn serve_healthz(listener: &UnixListener) -> Result<(), String> {
    let (mut stream, _) = listener.accept().map_err(|err| err.to_string())?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 512];
    loop {
        let n = stream.read(&mut tmp).map_err(|err| err.to_string())?;
        if n == 0 {
            break;
        }
        let chunk = tmp.get(..n).ok_or_else(|| "short read".to_owned())?;
        buf.extend_from_slice(chunk);
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&buf);
    if !request.starts_with("GET /healthz ") {
        return Err(format!("unexpected request {request}"));
    }
    let body = r#"{"status":"ok","pid":1}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn health(body: &str) -> Observation {
    Observation {
        stdout: body.to_owned(),
        stderr: String::new(),
        exit: 0,
        json: None,
        files: std::collections::BTreeMap::new(),
        tmux_argv: Vec::new(),
    }
}
