import { defineConfig } from "vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";

// React Compiler (stable 1.0) auto-memoizes components and hooks, so the data-heavy views re-render
// only on real changes without hand-written useMemo/useCallback noise. In plugin-react v6 it rides a
// Babel pass via the exported preset; React 19 ships the runtime, so no target override is needed.
// The agent's control/participant surfaces (default loopback bind, src/config.rs). Live mode talks
// to its own origin, so dev and preview proxy those paths to a running agent. Same-origin in the
// browser, no CORS, matching the production story where the agent itself serves this bundle.
// `/blobs` is top-level rather than under `/platform` because attachment bytes are served
// unauthenticated (an `<img src>` carries no bearer key), so it proxies as its own path.
const agentProxy = {
  "/control": "http://127.0.0.1:7777",
  "/platform": "http://127.0.0.1:7777",
  "/blobs": "http://127.0.0.1:7777",
};

// The embedded build (VITE_EMBEDDED, set by zuihitsu-console's build.rs) writes to that crate's
// `dist-embedded` dir: the build script passes an absolute path in `VITE_EMBEDDED_OUT`, and the
// crate-relative default covers a manual `VITE_EMBEDDED=true npm run build`. A plain
// `npm run build` for the dev checks cannot clobber the bytes the binary embeds, and vice versa.
// `process` is declared locally rather than pulling in @types/node just for this config.
declare const process: { env: Record<string, string | undefined> };
const embedded = process.env.VITE_EMBEDDED === "true";

export default defineConfig({
  plugins: [react(), babel({ presets: [reactCompilerPreset()] }), tailwindcss()],
  build: {
    outDir: embedded
      ? (process.env.VITE_EMBEDDED_OUT ?? "../crates/console/dist-embedded")
      : "dist",
  },
  server: { proxy: agentProxy },
  preview: { proxy: agentProxy },
});
