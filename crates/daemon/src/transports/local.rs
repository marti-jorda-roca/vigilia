//! The local transport: an owner-only Unix socket. Only processes of the same user may talk to
//! the service; they get `ClientContext::local()`.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use vigilia_protocol::{ClientContext, ErrorCode, Request, Response};

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
    let (read, mut write) = stream.into_split();
    let (out, mut out_rx) = mpsc::channel::<String>(32);
    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut reader = BufReader::new(read);
    let mut forward: Option<tokio::task::JoinHandle<()>> = None;
    let mut line = String::new();
    loop {
        line.clear();
        match (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_line(&mut line)
            .await
        {
            Ok(0) => break,
            Ok(_) if line.len() > MAX_LINE => {
                let resp = Response::err(None, ErrorCode::BadRequest, "request line too long");
                let _ = out.send(encode(&resp)).await;
                break;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(%error, "connection read failed");
                break;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        let req = match Request::parse(line.trim_end()) {
            Ok(req) => req,
            Err(e) => {
                let resp = Response::err(e.id, ErrorCode::BadRequest, e.message);
                if out.send(encode(&resp)).await.is_err() {
                    break;
                }
                continue;
            }
        };
        let Some(dispatched) = service.call(ClientContext::local(), req).await else {
            break;
        };
        if out.send(encode(&dispatched.response)).await.is_err() {
            break;
        }
        if let Some(mut sub) = dispatched.subscription {
            if let Some(previous) = forward.take() {
                previous.abort();
                service.subscriber_gone().await;
            }
            let out = out.clone();
            forward = Some(tokio::spawn(async move {
                while let Some(event) = sub.recv().await {
                    if out.send(encode(&event)).await.is_err() {
                        break;
                    }
                }
            }));
        }
    }
    if let Some(task) = forward {
        task.abort();
        service.subscriber_gone().await;
    }
    drop(out);
    let _ = writer.await;
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
