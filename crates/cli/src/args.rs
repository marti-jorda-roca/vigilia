//! Command-line arguments.

use clap::{Parser, Subcommand};
use vigilia_protocol::HumanDuration;

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
                extend: false
            }
        );
        assert_eq!(
            command(&["start", "2h", "--extend"]),
            Command::Start {
                duration: Some(two_hours),
                extend: true
            }
        );
        assert_eq!(
            command(&["start"]),
            Command::Start {
                duration: None,
                extend: false
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
