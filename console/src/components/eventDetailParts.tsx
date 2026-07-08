import { Fragment, type ReactNode } from "react";
import { Link } from "react-router-dom";

import { refName } from "../lib/model/events.ts";
import { statePath } from "../lib/nav/routes.ts";
import { Excerpt } from "../components/primitives.tsx";

/// A memory reference: the memory's name, a link into the State view at this event's seq when the
/// stream's `base` and the `seq` are known and the id names a memory, plain text otherwise.
export function Ref({
  id,
  nameById,
  base,
  seq,
}: {
  id: string;
  nameById: Map<string, string>;
  base?: string;
  seq?: number;
}) {
  const name = refName(id, nameById);
  const to = base != null && seq != null && nameById.has(id) ? statePath(base, seq, name) : null;
  if (!to) return <>{name}</>;
  return (
    <Link
      to={to}
      title="Open this memory in State, at this point in the timeline"
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {name}
    </Link>
  );
}

/// A comma-separated list of memory references, each a link under the same rules as [`Ref`].
export function RefList({
  ids,
  nameById,
  base,
  seq,
  empty = "—",
}: {
  ids: string[];
  nameById: Map<string, string>;
  base?: string;
  seq?: number;
  empty?: string;
}) {
  if (ids.length === 0) return <>{empty}</>;
  return (
    <>
      {ids.map((id, index) => (
        <Fragment key={index}>
          {index > 0 && ", "}
          <Ref id={id} nameById={nameById} base={base} seq={seq} />
        </Fragment>
      ))}
    </>
  );
}

export function Mono({ children }: { children: ReactNode }) {
  return <span className="break-all text-ink-soft">{children}</span>;
}

/// A long text body (a brief, a prompt template) — the content itself, not a JSON dump.
export function Prose({ children }: { children: string }) {
  return <Excerpt>{children}</Excerpt>;
}
