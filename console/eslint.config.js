import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import prettier from "eslint-config-prettier";

// Generated outputs (the ts-rs bindings and the wasm bundle) and the build dirs are not ours to lint.
export default tseslint.config(
  { ignores: ["dist", "dist-embedded", "src/types/**", "src/wasm/**"] },
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
    },
    rules: {
      ...reactHooks.configs["recommended-latest"].rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
    },
  },
  {
    // The regen.sh-adjacent Node scripts (e.g. the settings-metadata extractor) run on Node, not the
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
