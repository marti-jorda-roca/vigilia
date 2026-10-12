//! A blocking client for the local socket. Each request is one line out and one line back, so
//! no async runtime is needed; `status --watch` waits on the socket with a read timeout.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use vigilia_core::Paths;
use vigilia_protocol::{Call, Request, ServerMessage, Session, SessionResult};

use crate::error::CliError;

/// How long to wait for a service we just spawned. It normally listens within 50 ms.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    /// A line read only partly before a read timeout; the rest arrives on the next read.
    partial: Vec<u8>,
    next_id: u64,
}

impl Client {
    pub fn connect(socket: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
        Ok(Self {
            writer: stream.try_clone()?,
            reader: BufReader::new(stream),
            partial: Vec::new(),
            next_id: 0,
        })
    }

    /// Connects to the service, starting it first if it isn't running. Without socket
    /// activation (dev builds, or before `vigilia install`), this is how the service starts.
    pub fn connect_or_spawn(paths: &Paths) -> Result<Self, CliError> {
        match Self::connect(&paths.socket) {
            Err(e) if service_absent(&e) => {}
            other => return other.map_err(CliError::Unreachable),
        }
        spawn_service().map_err(CliError::Unreachable)?;
        let deadline = Instant::now() + SPAWN_TIMEOUT;
        let mut wait = Duration::from_millis(5);
        loop {
            std::thread::sleep(wait);
            match Self::connect(&paths.socket) {
                Err(e) if service_absent(&e) && Instant::now() < deadline => {
                    wait = (wait * 2).min(Duration::from_millis(100));
                }
                other => return other.map_err(CliError::Unreachable),
            }
        }
    }

    /// Sends `call` and waits for its response, skipping any events in between.
    pub fn call<T: DeserializeOwned>(&mut self, call: Call) -> Result<T, CliError> {
        self.next_id += 1;
        let id = self.next_id;
        let mut line = serde_json::to_string(&Request::new(id, call))
            .map_err(|e| CliError::BadReply(e.to_string()))?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(CliError::Unreachable)?;
        loop {
            match self.next_message(None)? {
                Some(ServerMessage::Response(resp)) if resp.id == Some(id) => {
                    return Ok(resp.into_result()?);
                }
                // The service couldn't read the request at all.
                Some(ServerMessage::Response(resp)) if resp.id.is_none() => {
                    return Err(resp.error.map_or_else(
                        || CliError::BadReply("response without an id".into()),
                        CliError::Refused,
                    ));
                }
                _ => {}
            }
        }
    }

    /// Calls a `session.*` method.
    pub fn session(&mut self, call: Call) -> Result<Option<Session>, CliError> {
        Ok(self.call::<SessionResult>(call)?.session)
    }

    /// The next message from the service. `Ok(None)` when `timeout` passes first.
    pub fn next_message(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<Option<ServerMessage>, CliError> {
        let timeout = timeout.map(|t| t.max(Duration::from_millis(1)));
        self.reader
            .get_ref()
            .set_read_timeout(timeout)
            .map_err(CliError::Unreachable)?;
        match self.reader.read_until(b'\n', &mut self.partial) {
            Ok(0) => Err(CliError::Disconnected),
            Ok(_) if self.partial.last() != Some(&b'\n') => Err(CliError::Disconnected),
            Ok(_) => {
                let msg = serde_json::from_slice(&self.partial)
                    .map_err(|e| CliError::BadReply(e.to_string()));
                self.partial.clear();
                msg.map(Some)
            }
            Err(e) if is_timeout(&e) => Ok(None),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(None),
            Err(e) => Err(CliError::Unreachable(e)),
        }
    }
}

/// Nothing is listening: no socket file yet, or a stale one left by a service that exited.
fn service_absent(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Starts `vigilia daemon` detached from this terminal. If two clients race, the instance lock
/// makes the second service exit straight away.
fn spawn_service() -> io::Result<()> {
    use std::os::unix::process::CommandExt;

    Command::new(std::env::current_exe()?)
        .arg("daemon")
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group, so Ctrl-C on `status --watch` doesn't reach it.
        .process_group(0)
        .spawn()
        .map(drop)
}
