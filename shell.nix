# Development shell providing a wasm C toolchain for the console's WASM build.
#
# The console compiles the agent's rusqlite-backed materializer to wasm32-unknown-unknown (see
# console/PLAN.md), so sqlite-wasm-rs must compile SQLite's C sources for that target. The default
# Nix clang wrapper injects the host glibc include paths, which a freestanding wasm compile must
# never see — it dies on `__GLIBC_USE` not being defined. The fix is to point cc-rs at an unwrapped
# clang for the wasm target only; the host C toolchain (mlua, rusqlite's bundled SQLite, sqlite-vec)
# stays on the standard wrapped compiler, untouched.
#
# Rust itself comes from the developer's rustup (the wasm32-unknown-unknown target is added with
# `rustup target add wasm32-unknown-unknown`); this shell deliberately does not pin a toolchain, so
# it composes with the existing setup rather than shadowing it.
{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = [
    # The unwrapped clang for the wasm C toolchain (see header comment).
    pkgs.llvmPackages.clang-unwrapped
    # Node runs console/scripts/extract-settings-metadata.mjs during `./console/regen.sh`.
    pkgs.nodejs
  ];

  # cc-rs reads CC_<target> (target with dashes as underscores). Scoping the override to
  # wasm32-unknown-unknown leaves every host C build on the standard wrapped compiler.
  CC_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang";
}
