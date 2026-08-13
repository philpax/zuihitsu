import { defineConfig } from "vitest/config";

// A standalone vitest config (vitest ignores vite.config.ts when this file exists), so the React
// Compiler babel pass and the Tailwind plugin never load under test. Pure functions in src/lib run
// under node; a component test opts into jsdom per file (`// @vitest-environment jsdom`) — without
// the compiler pass, which the components must not depend on for correctness anyway.

// `ImportMeta.url` is declared in the DOM lib, which this Node-side project deliberately does not
// load; the one property used is declared here, as `vite.config.ts` declares `process` locally rather
// than pulling in @types/node.
declare global {
  interface ImportMeta {
    url: string;
  }
}

// Where the wasm bundle sits, resolved here rather than in the setup file: this config always
// evaluates as a Node module, so `import.meta.url` is a `file:` URL, where under jsdom it is the
// document's `http:` origin and resolves to nothing readable. This file sits at the console root, so
// trimming its name yields the root the bundle is addressed from.
const consoleRoot = import.meta.url.slice(0, import.meta.url.lastIndexOf("/") + 1);
const wasmHref = `${consoleRoot}packages/wire/wasm/console_wasm_bg.wasm`;

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
    // The wasm bundle is initialised before any test runs, so the Rust definitions the console shares
    // — the reference parser, the token estimator — are the real ones under test as well as in the
    // browser. See `src/test/setupWasm.ts` for why it loads differently here.
    setupFiles: ["./src/test/setupWasm.ts"],
  },
  define: { __WASM_BUNDLE_HREF__: JSON.stringify(wasmHref) },
});
