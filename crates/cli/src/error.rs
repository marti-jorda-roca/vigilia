//! What can go wrong in a client command, and the exit code for each.

use std::io;

use vigilia_protocol::ErrorBody;

/// Exit codes scripts can rely on. `2` (usage error) comes from clap.
pub mod exit {
    pub const REFUSED: u8 = 1;
    pub const UNREACHABLE: u8 = 3;
    pub const NEEDS_ATTENTION: u8 = 4;
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The service answered with an error.
    #[error("{}", .0.message)]
    Refused(ErrorBody),
    #[error("could not reach the vigilia service: {0}")]
    Unreachable(io::Error),
    #[error("the vigilia service closed the connection")]
    Disconnected,
    #[error("unexpected reply from the vigilia service: {0}")]
    BadReply(String),
    #[error("the vigilia CLI is not supported on this OS yet")]
    Unsupported,
}

impl CliError {
    /// The code shown with the message, and in `--json` errors. Service errors keep the
    /// protocol's code.
    pub fn code(&self) -> String {
        match self {
            CliError::Refused(body) => serde_json::to_value(body.code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "internal".to_owned()),
            CliError::Unreachable(_) | CliError::Disconnected | CliError::BadReply(_) => {
                "unreachable".to_owned()
            }
            CliError::Unsupported => "unsupported".to_owned(),
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Refused(_) => exit::REFUSED,
            _ => exit::UNREACHABLE,
        }
    }
}

impl From<ErrorBody> for CliError {
    fn from(body: ErrorBody) -> Self {
        CliError::Refused(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vigilia_protocol::ErrorCode;

    #[test]
    fn codes_and_exit_codes() {
        let refused = CliError::Refused(ErrorBody {
            code: ErrorCode::SessionActive,
            message: "a session is already running".into(),
        });
        assert_eq!(refused.code(), "session_active");
        assert_eq!(refused.exit_code(), 1);
        let gone = CliError::Unreachable(io::ErrorKind::ConnectionRefused.into());
        assert_eq!(gone.code(), "unreachable");
        assert_eq!(gone.exit_code(), 3);
    }
}
