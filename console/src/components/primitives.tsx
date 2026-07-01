import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  SelectHTMLAttributes,
} from "react";

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

/// The console's one select shape, matching `TextInput` — used where a list collapses to a native
/// dropdown (the mobile conversation and memory pickers).
export function Select({ className = "", ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      {...rest}
      className={
        "w-full rounded-xs border border-line bg-paper px-2.5 py-2 text-sm text-ink focus:border-ink-faint focus:outline-none " +
        className
      }
    />
  );
}

/// A quoted block of the agent's own material — a brief, a prompt, a judge response, a raw payload —
/// set off on an oat ground behind a hairline. Scrolls internally past `maxHeight` so a long capture
/// never swallows the page.
export function Excerpt({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <pre
      className={
        "max-h-72 overflow-auto whitespace-pre-wrap border-l border-line bg-oat/40 px-3 py-2 font-mono text-xs leading-relaxed text-ink-soft " +
        className
      }
    >
      {children}
    </pre>
  );
}

/// A rule with a centered label — the seam between sessions, an entrance into a room: an event in the
/// flow of a transcript rather than a heading above it.
export function LabeledDivider({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={`flex items-center gap-3 ${className}`}>
      <span className="h-px flex-1 bg-line" />
      <span className="flex min-w-0 items-baseline gap-2 font-mono text-2xs">{children}</span>
      <span className="h-px flex-1 bg-line" />
    </div>
  );
}

/// A slim fill toward a budget — sage with headroom, clay past `warnAt` (where the budget looms).
export function Meter({
  fraction,
  warnAt = 0.8,
  className = "",
  title,
}: {
  fraction: number;
  warnAt?: number;
  className?: string;
  title?: string;
}) {
  return (
    <div className={`h-1 shrink-0 bg-oat ${className}`} title={title}>
      <div
        className={"h-1 " + (fraction >= warnAt ? "bg-clay" : "bg-sage")}
        style={{ width: `${Math.min(100, fraction * 100)}%` }}
      />
    </div>
  );
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
