//! The local transport: an owner-only Unix socket. Only processes of the same user may talk to
//! the service; they get `ClientContext::local()`.

use std::io;
use std::ops::ControlFlow;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use vigilia_protocol::{ClientContext, ErrorCode, Request, Response};

use crate::bus::Subscription;
use crate::service::ServiceHandle;

/// Longest request line accepted. Requests are tiny; this only stops a runaway client.
const MAX_LINE: usize = 64 * 1024;

/// Binds the socket at `path`, replacing a stale one. Call only while holding the instance lock.
pub fn bind(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    // The parent directory is already 0700; this is belt and braces.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Accepts connections forever, one task per connection.
pub async fn serve(listener: UnixListener, service: ServiceHandle) {
    // SAFETY: geteuid has no preconditions.
    let own_uid = unsafe { libc::geteuid() };
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::warn!(%error, "accept failed");
                continue;
            }
        };
        match stream.peer_cred() {
            Ok(cred) if peer_allowed(cred.uid(), own_uid) => {
                tokio::spawn(handle(stream, service.clone()));
            }
            Ok(cred) => tracing::warn!(uid = cred.uid(), "refused a connection from another user"),
            Err(error) => tracing::warn!(%error, "could not read peer credentials"),
        }
    }
}

fn peer_allowed(peer_uid: u32, own_uid: u32) -> bool {
    peer_uid == own_uid
}

async fn handle(stream: UnixStream, service: ServiceHandle) {
    let (read, write) = stream.into_split();
    let (out, out_rx) = mpsc::channel::<String>(32);
    let writer = tokio::spawn(write_lines(write, out_rx));

    let mut reader = BufReader::new(read);
    let mut forward: Option<JoinHandle<()>> = None;
    let mut line = String::new();
    while read_request_line(&mut reader, &mut line, &out).await {
        if line.trim().is_empty() {
            continue;
        }
        let ControlFlow::Continue(subscription) = answer(line.trim_end(), &service, &out).await
        else {
            break;
        };
        if let Some(sub) = subscription {
            stop_forwarding(forward.take(), &service).await;
            forward = Some(tokio::spawn(forward_events(sub, out.clone())));
        }
    }
    stop_forwarding(forward, &service).await;
    drop(out);
    let _ = writer.await;
}

/// Reads the next request line into `line`. Returns false when the connection should close: on
/// EOF, on a read error, or after replying to a line longer than `MAX_LINE`.
async fn read_request_line(
    reader: &mut BufReader<OwnedReadHalf>,
    line: &mut String,
    out: &mpsc::Sender<String>,
) -> bool {
    line.clear();
    match reader.take(MAX_LINE as u64 + 1).read_line(line).await {
        Ok(0) => false,
        Ok(_) if line.len() > MAX_LINE => {
            let resp = Response::err(None, ErrorCode::BadRequest, "request line too long");
            let _ = out.send(encode(&resp)).await;
            false
        }
        Ok(_) => true,
        Err(error) => {
            tracing::debug!(%error, "connection read failed");
            false
        }
    }
}

/// Replies to one request line. Breaks when the connection should close.
async fn answer(
    line: &str,
    service: &ServiceHandle,
    out: &mpsc::Sender<String>,
) -> ControlFlow<(), Option<Subscription>> {
    let (response, subscription) = match Request::parse(line) {
        Ok(req) => {
            let Some(dispatched) = service.call(ClientContext::local(), req).await else {
                return ControlFlow::Break(());
            };
            (dispatched.response, dispatched.subscription)
        }
        Err(e) => (Response::err(e.id, ErrorCode::BadRequest, e.message), None),
    };
    if out.send(encode(&response)).await.is_err() {
        return ControlFlow::Break(());
    }
    ControlFlow::Continue(subscription)
}

async fn write_lines(mut write: OwnedWriteHalf, mut lines: mpsc::Receiver<String>) {
    while let Some(line) = lines.recv().await {
        if write.write_all(line.as_bytes()).await.is_err() {
            break;
        }
    }
}

async fn forward_events(mut sub: Subscription, out: mpsc::Sender<String>) {
    while let Some(event) = sub.recv().await {
        if out.send(encode(&event)).await.is_err() {
            break;
        }
    }
}

async fn stop_forwarding(task: Option<JoinHandle<()>>, service: &ServiceHandle) {
    if let Some(task) = task {
        task.abort();
        service.subscriber_gone().await;
    }
}

fn encode<T: serde::Serialize>(msg: &T) -> String {
    let mut line = serde_json::to_string(msg).unwrap_or_else(|e| {
        serde_json::to_string(&Response::err(None, ErrorCode::Internal, e.to_string()))
            .expect("error responses serialize")
    });
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_same_user_is_allowed() {
        assert!(peer_allowed(501, 501));
        assert!(!peer_allowed(0, 501));
        assert!(!peer_allowed(502, 501));
    }
}
