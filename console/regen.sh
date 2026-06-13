#!/usr/bin/env bash
# Regenerate the console's two checked-in, Rust-derived artifacts:
#   1. console/src/types — the ts-rs TypeScript bindings (the wire contract).
#   2. console/src/wasm  — the wasm materializer bundle the replica folds events through.
# Both are committed so a frontend-only checkout needs no Rust toolchain; rerun this whenever a
# wire type or the materializer changes. Run from anywhere in the repo:  ./console/regen.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

# The wasm build compiles SQLite's C for wasm32, which needs the unwrapped clang the dev shell wires
# up (see shell.nix). Re-exec inside it if we are not already there.
if [[ -z "${IN_NIX_SHELL:-}" ]]; then
    exec nix-shell --run "$0"
fi

echo "==> ts-rs type bindings -> console/src/types"
cargo run -q -p zuihitsu-eval -- export-types console/src/types

echo "==> wasm materializer -> console/src/wasm"
cargo build -q -p console-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir console/src/wasm \
    target/wasm32-unknown-unknown/release/console_wasm.wasm

# Shrink the SQLite-bearing module before it lands in git history.
wasm-opt -Oz console/src/wasm/console_wasm_bg.wasm -o console/src/wasm/console_wasm_bg.wasm.opt
mv console/src/wasm/console_wasm_bg.wasm.opt console/src/wasm/console_wasm_bg.wasm

echo "==> done. Review and commit console/src/types and console/src/wasm."
