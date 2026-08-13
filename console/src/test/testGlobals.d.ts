// The console installs no `@types/node` — the browser bundle has no business reaching Node APIs, and
// the one config that needs `process` declares it locally (`vite.config.ts`). The test setup reads the
// wasm bundle off disk, so the single function it calls is declared the same way.
declare module "node:fs" {
  export function readFileSync(path: URL): Uint8Array;
}

/// The wasm bundle's `file:` URL, substituted by `vitest.config.ts`, which resolves it as a Node
/// module and so has one — the test environment may not.
declare const __WASM_BUNDLE_HREF__: string;
