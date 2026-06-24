//! Build the web console into `console/dist` so `rust_embed` can bake it into the binary and the agent
//! can serve it at its own root with no separate step. Auto-builds when a Node toolchain is present;
//! skips gracefully (leaving a placeholder page) when it is not, or when `ZUIHITSU_SKIP_CONSOLE` is
//! set — so a checkout without Node, or a build that does not want the console, still links.

use std::{path::Path, process::Command};

fn main() {
    // Rebuild the embedded console only when its sources change, not on every Rust edit.
    for path in [
        "console/src",
        "console/index.html",
        "console/package.json",
        "console/package-lock.json",
        "console/vite.config.ts",
        "console/tsconfig.json",
        "console/tsconfig.app.json",
        "build.rs",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=ZUIHITSU_SKIP_CONSOLE");

    // The embedded build's own output dir (vite.config keys it off VITE_EMBEDDED), separate from the
    // `dist` a plain `npm run build` writes, so the two never clobber each other. This is what the
    // binary embeds (see the `Console` rust-embed in src/http_server).
    let dist = Path::new("console/dist-embedded");

    if std::env::var_os("ZUIHITSU_SKIP_CONSOLE").is_some() {
        warn("ZUIHITSU_SKIP_CONSOLE is set; embedding a placeholder console");
        return ensure_placeholder(dist);
    }
    if !succeeds(Command::new("npm").arg("--version")) {
        warn(
            "npm not found; embedding a placeholder console (install Node, then rebuild, to embed the real one)",
        );
        return ensure_placeholder(dist);
    }

    // A fresh checkout has no node_modules; install before building.
    if !Path::new("console/node_modules").exists()
        && !succeeds(Command::new("npm").args(["--prefix", "console", "install"]))
    {
        warn("`npm install` failed; embedding a placeholder console");
        return ensure_placeholder(dist);
    }

    // VITE_EMBEDDED makes the console connect to its own origin and skip the landing (see App.tsx).
    let built = succeeds(
        Command::new("npm")
            .args(["--prefix", "console", "run", "build"])
            .env("VITE_EMBEDDED", "true"),
    );
    if !built {
        warn("the console build failed; embedding a placeholder console");
        ensure_placeholder(dist);
    }
}

fn warn(message: &str) {
    println!("cargo:warning=console: {message}");
}

fn succeeds(command: &mut Command) -> bool {
    command
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Guarantee `console/dist/index.html` exists so `rust_embed` always has a folder to embed. A real
/// build already on disk is kept; otherwise a minimal page that explains how to get the full console.
fn ensure_placeholder(dist: &Path) {
    if dist.join("index.html").exists() {
        return;
    }
    std::fs::create_dir_all(dist).expect("create console/dist");
    std::fs::write(dist.join("index.html"), PLACEHOLDER).expect("write placeholder index.html");
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
    <p>This build did not bundle the web console. Install Node and rebuild, or run
       <code>npm --prefix console run build</code> and rebuild, to embed it — or use the console's dev
       server against this agent.</p>
  </body>
</html>
"#;
