//! Newline framing in front of the rmcp stdio transport.
//!
//! rmcp skips a line that is not JSON and sends nothing back. A client that
//! sends non-JSON input gets a JSON-RPC parse error (`-32700`) instead.

use rmcp::ServiceExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::error::Error;
use crate::server::RigMcp;

const PARSE_ERROR: &[u8] =
    b"{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"Parse error\"}}\n";

/// Serve `server` on `input` and `output`.
///
/// A line that is not a JSON value is answered with JSON-RPC `-32700` and is
/// not forwarded. Valid JSON lines are forwarded unchanged.
///
/// # Errors
///
/// Returns [`Error::Serve`] when the session fails, and [`Error::Io`] on a
/// read or write error.
pub async fn serve_io<R, W>(server: RigMcp, input: R, output: W) -> Result<(), Error>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (rmcp_read, filter_write) = tokio::io::duplex(64 * 1024);
    let (copy_read, rmcp_write) = tokio::io::duplex(64 * 1024);
    let (tx, rx) = mpsc::unbounded_channel();
    let session = tokio::spawn(run_session(server, rmcp_read, rmcp_write));
    let filter = tokio::spawn(forward_json(input, filter_write, tx.clone()));
    let copy = tokio::spawn(copy_lines(copy_read, tx));
    let write = tokio::spawn(write_out(output, rx));
    session.await.map_err(|err| join_err(&err))??;
    filter.await.map_err(|err| join_err(&err))??;
    copy.await.map_err(|err| join_err(&err))??;
    write.await.map_err(|err| join_err(&err))??;
    Ok(())
}

async fn run_session<R, W>(server: RigMcp, read: R, write: W) -> Result<(), Error>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let running = server
        .serve((read, write))
        .await
        .map_err(|err| Error::Serve(err.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|err| Error::Serve(err.to_string()))?;
    Ok(())
}

async fn forward_json<R, W>(
    input: R,
    mut toward_server: W,
    tx: mpsc::UnboundedSender<Vec<u8>>,
) -> Result<(), Error>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut reader = tokio::io::BufReader::new(input);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = reader.read_until(b'\n', &mut buf).await?;
        if read == 0 {
            break;
        }
        if json_line(&buf) {
            toward_server.write_all(&buf).await?;
            if !buf.ends_with(b"\n") {
                toward_server.write_all(b"\n").await?;
            }
            toward_server.flush().await?;
        } else if tx.send(PARSE_ERROR.to_vec()).is_err() {
            break;
        }
    }
    Ok(())
}

async fn copy_lines<R>(input: R, tx: mpsc::UnboundedSender<Vec<u8>>) -> Result<(), Error>
where
    R: AsyncRead + Unpin + Send,
{
    let mut reader = tokio::io::BufReader::new(input);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = reader.read_until(b'\n', &mut buf).await?;
        if read == 0 {
            break;
        }
        if tx.send(buf.clone()).is_err() {
            break;
        }
    }
    Ok(())
}

async fn write_out<W>(mut output: W, mut rx: mpsc::UnboundedReceiver<Vec<u8>>) -> Result<(), Error>
where
    W: AsyncWrite + Unpin + Send,
{
    while let Some(bytes) = rx.recv().await {
        output.write_all(&bytes).await?;
        output.flush().await?;
    }
    Ok(())
}

fn json_line(buf: &[u8]) -> bool {
    let line = buf.strip_suffix(b"\n").unwrap_or(buf);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let Ok(text) = std::str::from_utf8(line) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(text).is_ok()
}

fn join_err(err: &tokio::task::JoinError) -> Error {
    Error::Message(err.to_string())
}
