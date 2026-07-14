//! Build the web console into `console/dist-embedded` so `rust_embed` can bake it into the binary
//! and the agent can serve it at its own root with no separate step.
//!
//! When the `console` feature is on (default), `build.rs` runs the full pipeline:
//!   1. ts-rs type export (shells out to `zuihitsu-frontend-types`'s `export-types` binary)
//!   2. settings metadata (the dependency-free Node script over the ts-rs-generated `.ts` files)
//!   3. wasm materialiser build (shells out to `cargo build -p console-wasm --target wasm32`)
//!   4. wasm-bindgen glue (in-process via `wasm-bindgen-cli-support`)
//!   5. wasm-opt (in-process via the `wasm-opt` crate)
//!   6. npm install (if `console/node_modules` is absent)
//!   7. npm run build (with `VITE_EMBEDDED=true`)
//!
//! Every step propagates failure — the build panics on any error. No placeholder fallback, no
//! silent degradation. If the console feature is off (`--no-default-features`), the pipeline is
//! skipped entirely, and a minimal placeholder `index.html` is written so the `RustEmbed` macro
//! always finds the folder.
//!
//! Generated artifacts (ts-rs types, wasm bundle) are written to `console/packages/wire/` — a
//! local npm package the console depends on as `@zuihitsu/wire`. This keeps them outside
//! `console/src/` so the `rerun-if-changed=console/src` watch does not see its own outputs and
//! trigger a rebuild loop.

use std::path::Path;

fn main() {
    // Rebuild when any pipeline input changes. The whole `console/src` tree is watched — generated
    // outputs live in `console/packages/wire/` (a separate package), so they do not trigger a
    // rebuild loop.
    for path in [
        "crates/frontend-types/src",
        "crates/core/src",
        "crates/console-wasm/src",
        "console/src",
        "console/index.html",
        "console/package.json",
        "console/package-lock.json",
        "console/vite.config.ts",
        "console/tsconfig.json",
        "console/tsconfig.app.json",
        "console/scripts",
        "build.rs",
        "Cargo.toml",
        "crates/frontend-types/Cargo.toml",
        "crates/console-wasm/Cargo.toml",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    let dist = Path::new("console/dist-embedded");

    #[cfg(feature = "console")]
    {
        build_console(dist);
    }

    // Whether or not the console feature is on, ensure the dist-embedded folder exists so the
    // RustEmbed macro always finds it. When the feature is on, the real Vite build already wrote
    // the files; when off, a placeholder is written.
    ensure_placeholder(dist);
}

#[cfg(feature = "console")]
fn build_console(dist: &Path) {
    use std::process::Command;

    let console_build_target = "target/console-build";
    let wire_dir = Path::new("console/packages/wire");

    // 1. ts-rs type export — shell out to the frontend-types binary in a separate target dir to
    //    avoid lock contention with the main build. The binary takes the output directory as its
    //    sole argument.
    let types_dir = wire_dir.join("types");
    std::fs::create_dir_all(&types_dir).unwrap_or_else(|error| {
        panic!(
            "build.rs: could not create {}: {error}",
            types_dir.display()
        )
    });
    run(
        Command::new("cargo").args([
            "run",
            "--locked",
            "-p",
            "zuihitsu-frontend-types",
            "--features",
            "ts",
            "--target-dir",
            console_build_target,
            "--",
            &types_dir.to_string_lossy(),
        ]),
        "ts-rs type export",
    );

    // 2. Settings metadata — the existing dependency-free Node script that parses the ts-rs-generated
    //    .ts files. Runs after ts-rs export, before the vite build.
    run(
        Command::new("node").arg("console/scripts/extract-settings-metadata.mjs"),
        "settings metadata generation",
    );

    // 3. Wasm materialiser build — shell out to cargo build for the wasm32 target in a separate
    //    target dir to avoid lock contention.
    run(
        Command::new("cargo").args([
            "build",
            "--locked",
            "-p",
            "console-wasm",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--target-dir",
            console_build_target,
        ]),
        "wasm materialiser build",
    );

    let wasm_input =
        Path::new(console_build_target).join("wasm32-unknown-unknown/release/console_wasm.wasm");

    // 4. wasm-bindgen glue — in-process via the library (no shell-out to the CLI).
    let wasm_out = wire_dir.join("wasm");
    std::fs::create_dir_all(&wasm_out).unwrap_or_else(|error| {
        panic!("build.rs: could not create {}: {error}", wasm_out.display())
    });
    wasm_bindgen_cli_support::Bindgen::new()
        .input_path(&wasm_input)
        .web(true)
        .expect("build.rs: wasm-bindgen web configuration failed")
        .typescript(true)
        .generate(&wasm_out)
        .unwrap_or_else(|error| {
            panic!("build.rs: wasm-bindgen failed: {error}");
        });

    // 5. wasm-opt — in-process via the Rust crate (builds Binaryen from C++ source on first use).
    let wasm_bg = wasm_out.join("console_wasm_bg.wasm");
    let wasm_opt_temp = wasm_out.join("console_wasm_bg.wasm.opt");
    wasm_opt::OptimizationOptions::new_optimize_for_size_aggressively()
        .run(&wasm_bg, &wasm_opt_temp)
        .unwrap_or_else(|error| {
            panic!("build.rs: wasm-opt failed: {error}");
        });
    std::fs::rename(&wasm_opt_temp, &wasm_bg)
        .unwrap_or_else(|error| panic!("build.rs: could not replace the wasm-opt output: {error}"));

    // 6. npm install — only if node_modules is absent (a fresh checkout).
    if !Path::new("console/node_modules").exists() {
        run(
            Command::new("npm").args(["--prefix", "console", "install"]),
            "npm install",
        );
    }

    // 7. npm run build with VITE_EMBEDDED=true — the Vite production build into dist-embedded.
    run(
        Command::new("npm")
            .args(["--prefix", "console", "run", "build"])
            .env("VITE_EMBEDDED", "true"),
        "npm run build (vite)",
    );

    // Verify the real build landed.
    if !dist.join("index.html").exists() {
        panic!(
            "build.rs: the console build completed but {} was not produced",
            dist.join("index.html").display()
        );
    }
}

/// Run a command, panicking with a clear context message on failure.
#[cfg(feature = "console")]
fn run(command: &mut std::process::Command, label: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("build.rs: could not spawn the {label} command: {error}");
    });
    if !status.success() {
        panic!(
            "build.rs: {label} failed with exit code {:?}",
            status.code()
        );
    }
}

/// Guarantee `console/dist-embedded/index.html` exists so `rust_embed` always has a folder to
/// embed. A real build already on disk is kept; otherwise a minimal placeholder page is written.
fn ensure_placeholder(dist: &Path) {
    if dist.join("index.html").exists() {
        return;
    }
    std::fs::create_dir_all(dist).expect("build.rs: create console/dist-embedded");
    std::fs::write(dist.join("index.html"), PLACEHOLDER)
        .expect("build.rs: write placeholder index.html");
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>zuihitsu</title>
    <style>
      body { font-family: ui-sans-serif, system-ui, sans-serif; color: #2b2a26; background: #f3f1ea;
             max-width: 36rem; margin: 18vh auto 0; padding: 0 1.5rem; line-height: 1.6; }
      h1 { font-weight: 500; } code { background: #e7e4da; padding: 0.1em 0.35em; border-radius: 3px; }
    </style>
  </head>
  <body>
    <h1>zuihitsu console not embedded</h1>
    <p>This build did not bundle the web console. Build with the <code>console</code> feature
       (the default) to embed it, or use the console's dev server against this agent.</p>
  </body>
</html>
"#;
