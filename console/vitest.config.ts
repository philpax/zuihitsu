import { defineConfig } from "vitest/config";

// A standalone vitest config (vitest ignores vite.config.ts when this file exists), so the React
// Compiler babel pass and the Tailwind plugin never load under test. Pure functions in src/lib run
// under node; a component test opts into jsdom per file (`// @vitest-environment jsdom`) — without
// the compiler pass, which the components must not depend on for correctness anyway.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
