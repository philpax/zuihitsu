/// <reference types="vite/client" />

interface ImportMetaEnv {
  /// "true" when the console is built into the agent binary (set by build.rs). See App.tsx.
  readonly VITE_EMBEDDED?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
