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

pub use args::{Cli, Command, DebugCommand, LidArg};
pub use error::{CliError, exit};

pub fn run(cli: Cli) -> ExitCode {
    let code = match cli.command {
        Command::Daemon => daemon(),
        Command::Recover => recover(),
        Command::Install { yes } => install(yes, false),
        Command::Uninstall { yes } => install(yes, true),
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

/// `vigilia recover` talks to no service: it takes the lock itself, so it works exactly
/// when the service is dead.
fn recover() -> u8 {
    use std::io::Write;
    match vigilia_daemon::recover_blocking() {
        Ok(outcome) if outcome.already_running => {
            let _ = writeln!(
                std::io::stdout().lock(),
                "The vigilia service is running and owns the session state; nothing to recover."
            );
            0
        }
        Ok(outcome) => {
            let _ = writeln!(
                std::io::stdout().lock(),
                "Recovered {} action(s); sleep behaviour is back to normal.",
                outcome.undone
            );
            if outcome.failed > 0 {
                exit::NEEDS_ATTENTION
            } else {
                0
            }
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr().lock(), "vigilia recover: {e}");
            exit::UNREACHABLE
        }
    }
}

/// Shows what would be installed and asks, unless `--yes`; then installs or uninstalls.
fn install(yes: bool, uninstall: bool) -> u8 {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    if !uninstall {
        let _ = writeln!(
            stdout,
            "This installs the LaunchAgents and this sudoers rule ({}):\n{}",
            vigilia_daemon::install::SUDOERS_PATH,
            vigilia_daemon::install::sudoers_rule(&user)
        );
    }
    if !yes {
        let _ = write!(
            stdout,
            "{}? [y/N] ",
            if uninstall { "Uninstall" } else { "Install" }
        );
        let _ = stdout.flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err()
            || !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        {
            let _ = writeln!(stdout, "Aborted.");
            return 0;
        }
    }
    let result = if uninstall {
        vigilia_daemon::install::uninstall()
    } else {
        let paths = match vigilia_core::Paths::for_user() {
            Ok(paths) => paths,
            Err(e) => {
                let _ = writeln!(std::io::stderr().lock(), "vigilia install: {e}");
                return exit::UNREACHABLE;
            }
        };
        if let Err(e) = paths.create_dirs() {
            let _ = writeln!(std::io::stderr().lock(), "vigilia install: {e}");
            return exit::UNREACHABLE;
        }
        let program = std::env::current_exe().unwrap_or_else(|_| "vigilia".into());
        vigilia_daemon::install::install(&program, &user, &paths)
    };
    match result {
        Ok(done) => {
            for line in done {
                let _ = writeln!(stdout, "{line}");
            }
            0
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr().lock(), "vigilia: {e}");
            exit::REFUSED
        }
    }
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
            lid,
        } => commands::start_or_extend_lid(&mut client, duration, lid)?,
        // Without a duration, clap rejects `--extend`.
        Command::Start { duration, lid, .. } => commands::start_lid(&mut client, duration, lid)?,
        Command::Debug {
            command: DebugCommand::SimulateLid { state },
        } => {
            commands::simulate_lid(&mut client, state.into())?;
            return Ok(0);
        }
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
        Command::Daemon
        | Command::Recover
        | Command::Install { .. }
        | Command::Uninstall { .. } => {
            unreachable!("handled in run")
        }
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
