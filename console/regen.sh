#!/usr/bin/env bash
# Regenerate the console's Rust-derived artifacts (all gitignored, built fresh by CI):
#   1. console/src/types — the ts-rs TypeScript bindings (the wire contract).
#   2. console/src/types/settings-metadata.ts — field descriptions + units, extracted from the
#      ts-rs bindings (so a settings `///` doc comment change surfaces in the editor).
#   3. console/src/wasm  — the wasm materializer bundle the replica folds events through.
# Run from anywhere in the repo:  ./console/regen.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

echo "==> ts-rs type bindings -> console/src/types"
cargo run -q -p zuihitsu-eval -- export-types console/src/types

echo "==> settings metadata -> console/src/types/settings-metadata.ts"
node console/scripts/extract-settings-metadata.mjs

echo "==> wasm materializer -> console/src/wasm"
cargo build -q -p console-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir console/src/wasm \
    target/wasm32-unknown-unknown/release/console_wasm.wasm

# Shrink the SQLite-bearing module before it lands in git history.
wasm-opt -Oz console/src/wasm/console_wasm_bg.wasm -o console/src/wasm/console_wasm_bg.wasm.opt
mv console/src/wasm/console_wasm_bg.wasm.opt console/src/wasm/console_wasm_bg.wasm

echo "==> done. The ts-rs types, settings metadata, and wasm bundle are in console/src/types and console/src/wasm (gitignored — built fresh by CI)."
