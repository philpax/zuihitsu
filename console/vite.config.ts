import { defineConfig } from "vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";

// React Compiler (stable 1.0) auto-memoizes components and hooks, so the data-heavy views re-render
// only on real changes without hand-written useMemo/useCallback noise. In plugin-react v6 it rides a
// Babel pass via the exported preset; React 19 ships the runtime, so no target override is needed.
// The agent's control/participant surfaces (default loopback bind, src/config.rs). Live mode talks
// to its own origin, so dev and preview proxy those paths to a running agent — same-origin in the
// browser, no CORS, matching the production story where the agent itself serves this bundle.
const agentProxy = {
  "/control": "http://127.0.0.1:7777",
  "/platform": "http://127.0.0.1:7777",
};

export default defineConfig({
  plugins: [react(), babel({ presets: [reactCompilerPreset()] }), tailwindcss()],
  server: { proxy: agentProxy },
  preview: { proxy: agentProxy },
});
