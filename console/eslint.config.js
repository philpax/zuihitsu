import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import betterTailwind from "eslint-plugin-better-tailwindcss";
import prettier from "eslint-config-prettier";

// Generated outputs (the ts-rs bindings and the wasm bundle live in the @zuihitsu/wire package,
// symlinked into node_modules) are not ours to lint, nor are our build dirs.
export default tseslint.config(
  { ignores: ["dist", "dist-embedded", "packages/wire/**", "node_modules/@zuihitsu/wire/**"] },
  js.configs.recommended,
  tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2023,
      globals: globals.browser,
    },
    plugins: {
      // react-hooks' recommended-latest set carries the React Compiler rule too, so the views stay
      // within the Rules of React the compiler relies on; we take its rules and register the plugin
      // here in flat-config form.
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
      "better-tailwindcss": betterTailwind,
    },
    settings: {
      // Tailwind v4 has no JS config; the plugin resolves the registered utilities by reading the
      // CSS-first entry point (the `@import "tailwindcss"` and `@theme` tokens live here), so it must
      // be pointed at app.css to know which classes exist and how they order.
      "better-tailwindcss": {
        entryPoint: "src/app.css",
      },
    },
    rules: {
      ...reactHooks.configs["recommended-latest"].rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      ...betterTailwind.configs.recommended.rules,
      // Prettier owns JSX formatting and the className strings are written single-line by hand; this
      // rule wants to hard-wrap long class lists across lines, which fights both, so we leave it off
      // and keep only the semantic Tailwind checks (ordering, duplicates, conflicts, canonicalisation).
      "better-tailwindcss/enforce-consistent-line-wrapping": "off",
      // The console mixes a few hand-written CSS classes (defined in app.css) in among the utilities:
      // highlight.js' `hljs*` hooks, the range-input `scrubber`, the linked-turn `turn-linked` marker,
      // and KaTeX's `katex*` classes. They are not Tailwind utilities, so exempt them by name.
      "better-tailwindcss/no-unknown-classes": [
        "error",
        { ignore: ["^hljs", "^scrubber$", "^turn-linked$", "^katex"] },
      ],
    },
  },
  {
    // The build.rs-adjacent Node scripts (e.g. the settings-metadata extractor) run on Node, not the
    // browser, so they get the Node globals rather than the browser set.
    files: ["scripts/**/*.{js,mjs}"],
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: "module",
      globals: globals.node,
    },
  },
  prettier,
);
