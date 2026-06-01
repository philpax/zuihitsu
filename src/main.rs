//! The zuihitsu binary. With the `sqlite` feature it is the operator control CLI (see [`cli`]);
//! the richer server (the agent loop, platform clients, the debugger) lands in later stages.

use std::process::ExitCode;

#[cfg(feature = "sqlite")]
mod cli;

#[cfg(feature = "sqlite")]
fn main() -> ExitCode {
    cli::run()
}

#[cfg(not(feature = "sqlite"))]
fn main() -> ExitCode {
    eprintln!("zuihitsu was built without the `sqlite` feature; the server CLI is unavailable.");
    ExitCode::FAILURE
}
