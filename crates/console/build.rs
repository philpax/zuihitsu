//! Build the web console into `dist-embedded` so [`crate::Console`] (rust-embed) can bake it into
//! the binary and the agent or eval binary can serve it at its own root with no separate step.
//!
//! This crate owns the full pipeline, run unconditionally whenever the crate is compiled (it has no
//! features — including it means you want the console):
//!   1. ts-rs type export (shells out to `zuihitsu-frontend-types`'s `export-types` binary)
//!   2. settings metadata (generated inside the export-types binary via the `SettingsMetadata`
//!      proc-macro derive, which extracts `///` doc comments at compile time)
//!   3. wasm materialiser build (shells out to `cargo build -p console-wasm --target wasm32`)
//!   4. wasm-bindgen glue (in-process via `wasm-bindgen-cli-support`)
//!   5. wasm-opt (in-process via the `wasm-opt` crate)
//!   6. npm ci (when `console/node_modules` is missing or stale and no dev server is running)
//!   7. npm run build (with `VITE_EMBEDDED=true` and `VITE_EMBEDDED_OUT` pointing at `dist-embedded`)
//!
//! Every step propagates failure — the build panics on any error. No placeholder fallback: the final
//! assertion (panic if `index.html` is missing) guarantees `dist-embedded` is fully populated
//! before the lib compiles, so `RustEmbed` always finds a real folder. A build that should not run
//! the pipe-line (the workspace's rust CI job, which has no frontend toolchain) must exclude this
//! crate (`--exclude zuihitsu-console`), exactly as it excludes `zuihitsu-eval`.
//!
//! Generated artifacts (ts-rs types, wasm bundle) are written to `console/packages/wire/` — a
//! local npm package the console depends on as `@zuihitsu/wire`. This keeps them outside
//! `console/src/` so the `rerun-if-changed=console/src` watch does not see its own outputs and
//! trigger a rebuild loop.

use std::{path::Path, time::SystemTime};

#[path = "build/install.rs"]
mod install;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/console must live two levels below the workspace root");
    let console_dir = root.join("console");

    // The build script's own home (this crate) and the workspace root both feed the pipeline: its
    // own inputs are manifest-relative, the pipeline's inputs are root-relative. Rebuild when any
    // of them changes. The whole `console/src` tree is watched — generated outputs live in
    // `console/packages/wire/` (a separate package), so they do not trigger a rebuild loop.
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("build.rs").display()
    );
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
        "Cargo.toml",
        "crates/frontend-types/Cargo.toml",
        "crates/console-wasm/Cargo.toml",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }

    // Do not watch `node_modules` (its mtimes change on install and must not trigger rebuilds) or
    // the crate's own `dist-embedded` output.

    build_console(root, &console_dir, &manifest.join("dist-embedded"));
}

fn build_console(root: &Path, console_dir: &Path, dist: &Path) {
    use std::process::Command;

    let console_build_target = root.join("target/console-build");
    let wire_dir = console_dir.join("packages/wire");

    // 1. ts-rs type export — shell out to the frontend-types binary in a separate target dir to
    //    avoid lock contention with the main build. The binary takes the output directory as its
    //    sole argument. Pass the target dir as an absolute path so the working directory of this
    //    build script (wherever cargo runs it from) cannot re-anchor it.
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
            &console_build_target.to_string_lossy(),
            "--",
            &types_dir.to_string_lossy(),
        ]),
        "ts-rs type export",
    );

    // 2. Settings metadata — generated inside the export-types binary (see the module docs).

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
            &console_build_target.to_string_lossy(),
        ]),
        "wasm materialiser build",
    );

    let wasm_input = console_build_target.join("wasm32-unknown-unknown/release/console_wasm.wasm");

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

    // 6. npm dependencies — install only when the tree is missing or stale, and never under a live
    //    dev server (npm ci deletes the tree, which would tear down the server's HMR session).
    //    Freshness by `node_modules`' directory mtime vs `package.json`/`package-lock.json`.
    ensure_dependencies(console_dir);

    // 7. npm run build with VITE_EMBEDDED=true — the Vite production build into dist-embedded. The
    //    output dir is passed in as an absolute path (VITE_EMBEDDED_OUT) so the npm working
    //    directory cannot re-anchor it.
    run(
        Command::new("npm")
            .args([
                "--prefix",
                console_dir.to_string_lossy().as_ref(),
                "run",
                "build",
            ])
            .env("VITE_EMBEDDED", "true")
            .env("VITE_EMBEDDED_OUT", dist),
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

/// Make sure `console/node_modules` is present and fresh relative to the npm manifest and lockfile
/// — `npm ci` when stale or missing, a warning instead when a live dev server would be clobbered.
fn ensure_dependencies(console_dir: &Path) {
    let node_modules = console_dir.join("node_modules");
    let tree_mtime = std::fs::metadata(&node_modules)
        .and_then(|m| m.modified())
        .ok();
    let lock = console_dir.join("package-lock.json");
    let manifest = console_dir.join("package.json");
    let Some(lock_mtime) = mtime(&lock) else {
        panic!(
            "build.rs: {} is missing; the console's npm manifest has no lockfile. Run `npm install` in console/ and commit the lockfile, or build without the console feature.",
            lock.display()
        );
    };
    let manifest_mtime =
        mtime(&manifest).unwrap_or_else(|| panic!("build.rs: {} is missing", manifest.display()));
    let pidfile = node_modules.join(".zuihitsu-vite.pid");
    let dev_server = install::dev_server_active(&pidfile);
    match install::decide(tree_mtime, lock_mtime, manifest_mtime, dev_server) {
        install::Mode::Fresh => {}
        install::Mode::InstallNpmCi => {
            run(
                std::process::Command::new("npm").args([
                    "--prefix",
                    console_dir.to_string_lossy().as_ref(),
                    "ci",
                    "--no-audit",
                    "--no-fund",
                ]),
                "npm ci",
            );
        }
        install::Mode::SkipDevServerActive => {
            println!(
                "cargo:warning=the console dev server is running ({}); leaving console/node_modules untouched. Stop `npm run dev` to let the build reinstall stale dependencies.",
                pidfile.display()
            );
        }
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Run a command, panicking with a clear context message on failure.
fn run(command: &mut std::process::Command, label: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("build.rs: could not spawn the {label} command: {error}");
    });
    if !status.success() {
        panic!(
            "build.rs: {label} failed with exit code {:?}. If console/package.json changed without console/package-lock.json, run `npm install` in console/ and commit the updated lockfile, or build without the console feature.",
            status.code()
        );
    }
}
