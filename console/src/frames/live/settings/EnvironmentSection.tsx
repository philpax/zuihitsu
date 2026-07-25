import { useEffect, useState } from "react";

import type { LiveConnection } from "../../../lib/api/live.ts";
import { type ConfigTree, getConfig } from "../../../lib/api/config.ts";
import { Eyebrow } from "../../../components/primitives.tsx";
import { label } from "./settingsUtilities.ts";
import { ConfigFields } from "./fields.tsx";

/// The environmental TOML config, read-only.
export function EnvironmentSection({ connection }: { connection: LiveConnection }) {
  const [config, setConfig] = useState<ConfigTree | null | "unavailable">(null);

  useEffect(() => {
    let cancelled = false;
    getConfig(connection).then(
      (value) => !cancelled && setConfig(value),
      () => !cancelled && setConfig("unavailable"),
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

  if (config === null) {
    return <p className="py-12 text-center text-sm text-ink-faint">Loading the environment…</p>;
  }
  if (config === "unavailable") {
    return (
      <p className="py-12 text-center text-sm text-ink-faint">
        The environment is not available from this host.
      </p>
    );
  }
  return (
    <div>
      <p className="max-w-prose text-sm/relaxed text-ink-soft">
        The TOML config this instance booted from — read-only here (it is read at startup, not from
        the log). Secrets are redacted: API keys show as counts, MCP env as its variable names.
      </p>
      <div className="mt-6 flex flex-col gap-7">
        {Object.entries(config).map(([group, value]) => (
          <div key={group}>
            <Eyebrow>{label(group)}</Eyebrow>
            <div className="mt-3">
              <ConfigFields value={value} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
