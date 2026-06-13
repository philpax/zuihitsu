import { defineConfig } from "vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";

// React Compiler (stable 1.0) auto-memoizes components and hooks, so the data-heavy views re-render
// only on real changes without hand-written useMemo/useCallback noise. In plugin-react v6 it rides a
// Babel pass via the exported preset; React 19 ships the runtime, so no target override is needed.
export default defineConfig({
  plugins: [
    react(),
    babel({ presets: [reactCompilerPreset()] }),
    tailwindcss(),
  ],
});
