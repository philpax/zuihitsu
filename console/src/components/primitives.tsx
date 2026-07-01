import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";

/// The small mono, upper-case, wide-tracked label that titles a section or stands as an eyebrow —
/// the quiet structural device used throughout, in place of heavier headings.
export function Eyebrow({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <span className={`font-mono text-2xs uppercase tracking-widest text-ink-soft ${className}`}>
      {children}
    </span>
  );
}

/// The console's one action shape: a hairline outline that warms to clay on hover. `primary` marks
/// the view's main verb (save, send, run) with a stronger stroke; everything else stays quiet. One
/// component so every view's buttons agree, instead of each re-deriving the class string.
export function Button({
  primary = false,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { primary?: boolean }) {
  return (
    <button
      {...rest}
      className={
        "rounded-xs border px-4 py-2 font-mono text-xs text-ink transition-colors enabled:hover:border-clay enabled:hover:text-clay disabled:opacity-45 " +
        (primary ? "border-ink-faint " : "border-line-strong ") +
        className
      }
    />
  );
}

/// The console's one text-input shape — hairline border, transparent ground, mono text — shared so
/// the sidebar fields, the landing, and the settings editor agree.
export function TextInput({ className = "", ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...rest}
      className={
        "w-full rounded-xs border border-line bg-transparent px-2.5 py-1.5 font-mono text-xs text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none " +
        className
      }
    />
  );
}

/// A quiet line of guidance under a control; `tone="error"` turns it clay for a failure that needs
/// acting on. Sized to be readable — hints are content, not chrome.
export function Hint({
  children,
  tone = "quiet",
  className = "",
}: {
  children: ReactNode;
  tone?: "quiet" | "error";
  className?: string;
}) {
  return (
    <span
      className={
        "font-mono text-xs " + (tone === "error" ? "text-clay " : "text-ink-faint ") + className
      }
    >
      {children}
    </span>
  );
}

/// A faint middot separating inline metadata.
export function Dot() {
  return <span className="text-ink-faint/45">·</span>;
}

/// A disclosure toggle — the `▸ label` / `▾ label` affordance used for briefs, deliberations,
/// prompts, and the API reference. One component so the arrow, spacing, and hover behavior agree
/// everywhere. `summary` is the always-visible row beside the label (a count, a duration).
export function Disclosure({
  open,
  onToggle,
  label,
  summary,
  className = "",
}: {
  open: boolean;
  onToggle: () => void;
  label: ReactNode;
  summary?: ReactNode;
  className?: string;
}) {
  return (
    <button
      onClick={onToggle}
      className={
        "flex items-baseline gap-2 text-left font-mono text-xs text-ink-faint transition-colors hover:text-ink-soft " +
        className
      }
    >
      <span aria-hidden className="inline-block w-3 shrink-0 text-center">
        {open ? "▾" : "▸"}
      </span>
      <span>{label}</span>
      {summary !== undefined && <span className="text-ink-faint/70">{summary}</span>}
    </button>
  );
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
