//! The `vigilia` command.
//!
//! Client commands talk to the service over its local socket and start it when it isn't
//! running; `vigilia daemon` is the service itself. Exit codes: `0` OK, `1` the service refused
//! the request, `2` usage error, `3` the service couldn't be reached, `4` the session needs
//! attention.

mod args;
mod error;
#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "only errors are printed until the client runs there"
    )
)]
mod output;

#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub mod commands;
#[cfg(unix)]
pub mod watch;

use std::process::ExitCode;

pub use args::{Cli, Command};
pub use error::{CliError, exit};

pub fn run(cli: Cli) -> ExitCode {
    let code = match cli.command {
        Command::Daemon => daemon(),
        command => {
            let plain_start = matches!(command, Command::Start { extend: false, .. });
            client_command(command, cli.json).unwrap_or_else(|e| {
                let hint = (plain_start && e.code() == "session_active").then_some(START_HINT);
                output::print_error(&e, cli.json, hint)
            })
        }
    };
    ExitCode::from(code)
}

fn daemon() -> u8 {
    match vigilia_daemon::run_blocking() {
        Ok(()) => 0,
        Err(e) => {
            use std::io::Write;
            let _ = writeln!(std::io::stderr().lock(), "vigilia daemon: {e}");
            exit::UNREACHABLE
        }
    }
}

#[cfg(unix)]
fn client_command(command: Command, json: bool) -> Result<u8, CliError> {
    use std::io::IsTerminal;

    let paths = vigilia_core::Paths::for_user().map_err(CliError::Unreachable)?;
    let mut client = client::Client::connect_or_spawn(&paths)?;
    let session = match command {
        Command::Start {
            duration: Some(duration),
            extend: true,
        } => commands::start_or_extend(&mut client, duration)?,
        // Without a duration, clap rejects `--extend`.
        Command::Start { duration, .. } => commands::start(&mut client, duration)?,
        Command::Extend { duration } => commands::extend(&mut client, duration)?,
        Command::Stop => commands::stop(&mut client)?,
        Command::Status { watch: false } => commands::status(&mut client)?,
        Command::Status { watch: true } => {
            let stdout = std::io::stdout();
            let mode = match (json, stdout.is_terminal()) {
                (true, _) => watch::Mode::Json,
                (false, true) => watch::Mode::Live,
                (false, false) => watch::Mode::Lines,
            };
            watch::watch(&mut client, mode, &mut stdout.lock())?;
            return Ok(0);
        }
        Command::Daemon => unreachable!("handled in run"),
    };
    Ok(output::print_session(session.as_ref(), json))
}

#[cfg(not(unix))]
fn client_command(_command: Command, _json: bool) -> Result<u8, CliError> {
    Err(CliError::Unsupported)
}

/// Shown when a plain `start` finds a session already running.
const START_HINT: &str = "`vigilia extend <duration>` adds time to it, and \
    `vigilia start <duration> --extend` makes it last at least that long";
