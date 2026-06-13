import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import prettier from "eslint-config-prettier";

// Generated outputs (the ts-rs bindings and the wasm bundle) and the build dir are not ours to lint.
export default tseslint.config(
  { ignores: ["dist", "src/types/**", "src/wasm/**"] },
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
  prettier,
);
