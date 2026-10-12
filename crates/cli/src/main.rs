use std::process::ExitCode;

use clap::Parser;
use vigilia_cli::Cli;

fn main() -> ExitCode {
    vigilia_cli::run(Cli::parse())
}
