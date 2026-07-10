import { defineConfig } from "vitest/config";

// A standalone vitest config (vitest ignores vite.config.ts when this file exists), so the React
// Compiler babel pass and the Tailwind plugin never load under test — the suite covers pure
// functions in src/lib only, which need no transforms.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
