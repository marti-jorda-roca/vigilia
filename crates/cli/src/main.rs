use std::process::ExitCode;

use clap::Parser;
use vigilia::Cli;

fn main() -> ExitCode {
    vigilia::run(Cli::parse())
}
