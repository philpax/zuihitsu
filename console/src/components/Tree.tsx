import { type ReactNode } from "react";

/// The readable fallback for a payload with no bespoke case: nested label/value rows rather than a
/// raw code block, so even an unforeseen event type stays legible.
export function Tree({ value }: { value: unknown }) {
  if (value === null || value === undefined) return <span className="text-ink-faint">—</span>;
  if (Array.isArray(value)) {
    if (value.length === 0) return <span className="text-ink-faint">(none)</span>;
    return (
      <div className="flex flex-col gap-1">
        {value.map((item, index) => (
          <Tree key={index} value={item} />
        ))}
      </div>
    );
  }
  if (typeof value === "object") {
    return (
      <Fields>
        {Object.entries(value as Record<string, unknown>).map(([key, child]) => (
          <Field key={key} label={key}>
            <Tree value={child} />
          </Field>
        ))}
      </Fields>
    );
  }
  return <span className="text-ink">{String(value)}</span>;
}

export function Fields({ children }: { children: ReactNode }) {
  return <div className="flex flex-col font-mono text-2xs text-ink-soft">{children}</div>;
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[6rem_1fr] gap-3 py-0.5">
      <span className="text-ink-faint">{label}</span>
      <span className="leading-relaxed">{children}</span>
    </div>
  );
}
