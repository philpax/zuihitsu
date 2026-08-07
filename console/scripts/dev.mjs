//! The wrapped `npm run dev`: record this wrapper's pid in a pidfile inside `node_modules` (targeting
//! a directory the build pipeline already reads), spawn the local Vite binary in its place, and clear
//! the pidfile when the dev server stops.
//!
//! `zuihitsu-console`'s build script reads the pidfile to decide whether `npm ci` is safe: `npm ci`
//! deletes `node_modules`, which would tear down a live HMR session under a running Vite server. The
//! recorded pid is the wrapper's, not Vite's, and the wrapper stays alive for as long as Vite runs, so
//! pid liveness is dev-server liveness.

import { spawn } from "node:child_process";
import { rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const consoleDir = dirname(dirname(fileURLToPath(import.meta.url)));
const pidfile = join(consoleDir, "node_modules", ".zuihitsu-vite.pid");

writeFileSync(pidfile, String(process.pid));

const viteBin = join(
  consoleDir,
  "node_modules",
  ".bin",
  process.platform === "win32" ? "vite.cmd" : "vite",
);

const cleanup = () => {
  try {
    rmSync(pidfile, { force: true });
  } catch {
    // The pidfile may already be gone; nothing to do.
  }
};

const child = spawn(viteBin, process.argv.slice(2), {
  stdio: "inherit",
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    child.kill(signal);
  });
}

child.on("exit", (code, signal) => {
  cleanup();
  if (signal !== null) {
    process.kill(process.pid, signal);
  } else if (code !== null && code !== 0) {
    process.exitCode = code;
  }
});
