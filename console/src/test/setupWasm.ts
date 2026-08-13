// Initialise the wasm bundle for every test file, so a test exercises the same Rust the browser runs.
//
// The console's shared definitions — the reference parser, the token estimator — live in Rust and cross
// once, at the `Replica` boundary. A test that stubbed them would be asserting against a TypeScript
// re-implementation of the rule under test, which is the drift the single crossing exists to prevent.
//
// The browser reaches the module by URL and `fetch`; neither is available under vitest, so the bytes
// are read off disk and handed to the synchronous initialiser instead. `__wbg_init` returns early once
// the module is live, so the lazy `ensureWasm` every `Replica` awaits becomes a no-op rather than a
// second instantiation.

import { readFileSync } from "node:fs";
import { initSync } from "@zuihitsu/wire/wasm/console_wasm.js";

initSync({ module: readFileSync(new URL(__WASM_BUNDLE_HREF__)) });
