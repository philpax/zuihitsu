//! The zuihitsu binary. Run with no subcommand (with the `serve` feature) it boots the long-running
//! HTTP server that hosts the agent (see [`serve`]); with a subcommand it is the operator CLI (see
//! [`cli`]), which targets that server's API. Without `sqlite` it does nothing useful.

use std::process::ExitCode;

#[cfg(feature = "sqlite")]
mod cli;
#[cfg(feature = "serve")]
mod serve;

#[cfg(feature = "sqlite")]
fn main() -> ExitCode {
    cli::run()
}

#[cfg(not(feature = "sqlite"))]
fn main() -> ExitCode {
    eprintln!("zuihitsu was built without the `sqlite` feature; the server CLI is unavailable.");
    ExitCode::FAILURE
}
