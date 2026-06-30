import type { ReactNode } from "react";

/// The small mono, upper-case, wide-tracked label that titles a section or stands as an eyebrow —
/// the quiet structural device used throughout, in place of heavier headings.
export function Eyebrow({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <span className={`font-mono text-2xs uppercase tracking-widest text-ink-soft ${className}`}>
      {children}
    </span>
  );
}

/// A faint middot separating inline metadata.
export function Dot() {
  return <span className="text-ink-faint/45">·</span>;
}

/// The clay-accented checkbox used for the console's toggles. Given a `label`, it wraps the box in a
/// clickable mono label; given none, it renders the bare input for callers that supply their own
/// label row (a `<label>` cannot nest another). `onChange` hands back the boolean directly.
export function Checkbox({
  checked,
  onChange,
  label,
  className = "",
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: ReactNode;
  className?: string;
}) {
  const input = (
    <input
      type="checkbox"
      checked={checked}
      onChange={(event) => onChange(event.target.checked)}
      className="accent-clay"
    />
  );
  if (label === undefined) return input;
  return (
    <label
      className={`flex cursor-pointer items-center gap-2 font-mono text-2xs text-ink-faint ${className}`}
    >
      {input}
      {label}
    </label>
  );
}
