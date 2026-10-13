//! Command-line arguments.

use clap::{Parser, Subcommand, ValueEnum};
use vigilia_protocol::{HumanDuration, LidState};

/// Keep this machine awake for a set time.
///
/// Durations are one or more <number><unit> groups, units d, h, m, s: `90s`, `45m`, `1h30m`.
#[derive(Debug, Parser)]
#[command(name = "vigilia", version)]
pub struct Cli {
    /// Print results as JSON, in the protocol's shape. Errors go to stderr as JSON too.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Start a session that ends after <DURATION>, or runs until `vigilia stop` without one.
    Start {
        duration: Option<HumanDuration>,
        /// If a session is already running, make it last at least <DURATION> from now instead
        /// of failing.
        #[arg(long, requires = "duration")]
        extend: bool,
        /// Keep the machine awake with the lid closed too (macOS, needs `vigilia install`).
        #[arg(long)]
        lid: bool,
    },
    /// Add <DURATION> to the running session.
    Extend { duration: HumanDuration },
    /// End the session now. Succeeds when no session is running.
    Stop,
    /// Show the current session.
    Status {
        /// Keep running and show the countdown until Ctrl-C.
        #[arg(long)]
        watch: bool,
    },
    /// Run the service in the foreground. Client commands start it when needed.
    Daemon,
    /// Undo anything a crashed session left behind and restore normal sleep. This is the
    /// backstop entry point; it is also safe to run by hand.
    Recover,
    /// Install the `LaunchAgents` and the sudoers rule for lid-closed sessions (macOS).
    /// Shows the rule and asks first, unless `--yes` is given.
    Install {
        #[arg(long)]
        yes: bool,
    },
    /// Remove what `vigilia install` installed.
    Uninstall {
        #[arg(long)]
        yes: bool,
    },
    /// Debugging helpers.
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum DebugCommand {
    /// Pretend the lid closed or opened.
    SimulateLid { state: LidArg },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LidArg {
    Close,
    Open,
}

impl From<LidArg> for LidState {
    fn from(arg: LidArg) -> Self {
        match arg {
            LidArg::Close => LidState::Closed,
            LidArg::Open => LidState::Open,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("vigilia").chain(args.iter().copied()))
    }

    fn command(args: &[&str]) -> Command {
        parse(args).unwrap().command
    }

    #[test]
    fn parses_every_command() {
        let two_hours = HumanDuration::from_secs(7_200);
        assert_eq!(
            command(&["start", "2h"]),
            Command::Start {
                duration: Some(two_hours),
                extend: false,
                lid: false
            }
        );
        assert_eq!(
            command(&["start", "2h", "--extend"]),
            Command::Start {
                duration: Some(two_hours),
                extend: true,
                lid: false
            }
        );
        assert_eq!(
            command(&["start"]),
            Command::Start {
                duration: None,
                extend: false,
                lid: false
            }
        );
        assert_eq!(
            command(&["extend", "1h30m"]),
            Command::Extend {
                duration: HumanDuration::from_secs(5_400)
            }
        );
        assert_eq!(command(&["stop"]), Command::Stop);
        assert_eq!(command(&["status"]), Command::Status { watch: false });
        assert_eq!(
            command(&["status", "--watch"]),
            Command::Status { watch: true }
        );
        assert_eq!(command(&["daemon"]), Command::Daemon);
    }

    #[test]
    fn parses_the_step_2_commands() {
        let two_hours = HumanDuration::from_secs(7_200);
        assert_eq!(
            command(&["start", "2h", "--lid"]),
            Command::Start {
                duration: Some(two_hours),
                extend: false,
                lid: true
            }
        );
        assert_eq!(command(&["recover"]), Command::Recover);
        assert_eq!(command(&["install"]), Command::Install { yes: false });
        assert_eq!(
            command(&["uninstall", "--yes"]),
            Command::Uninstall { yes: true }
        );
        assert_eq!(
            command(&["debug", "simulate-lid", "close"]),
            Command::Debug {
                command: DebugCommand::SimulateLid {
                    state: LidArg::Close
                }
            }
        );
        assert_eq!(
            command(&["debug", "simulate-lid", "open"]),
            Command::Debug {
                command: DebugCommand::SimulateLid {
                    state: LidArg::Open
                }
            }
        );
    }

    #[test]
    fn json_is_accepted_before_or_after_the_command() {
        assert!(parse(&["--json", "status"]).unwrap().json);
        assert!(parse(&["stop", "--json"]).unwrap().json);
        assert!(!parse(&["stop"]).unwrap().json);
    }

    #[test]
    fn bad_durations_are_usage_errors() {
        for bad in ["30", "2x", "", "h"] {
            let err = parse(&["start", bad]).unwrap_err();
            assert_eq!(err.exit_code(), 2, "{bad}");
        }
        assert!(parse(&["extend"]).is_err());
    }

    #[test]
    fn extend_flag_needs_a_duration() {
        let err = parse(&["start", "--extend"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }
}
