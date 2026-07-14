//! The `export-types` binary: writes the TypeScript wire-contract types into a directory.
//!
//! Invoked by the main crate's `build.rs` as:
//! `cargo run -p zuihitsu-frontend-types --features ts -- export-types <dir>`

#[cfg(feature = "ts")]
fn main() -> std::process::ExitCode {
    let dir: std::path::PathBuf = std::env::args()
        .nth(1)
        .expect("usage: export-types <dir>")
        .into();
    if !dir.is_dir() {
        eprintln!("export-types: {} is not a directory", dir.display());
        return std::process::ExitCode::FAILURE;
    }
    match zuihitsu_frontend_types::export_types(&dir) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("export-types: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "ts"))]
fn main() {
    eprintln!("export-types: the `ts` feature is required — build with `--features ts`");
    std::process::exit(1);
}
