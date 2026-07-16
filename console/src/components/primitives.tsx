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
    <span
      className={`font-mono text-2xs font-medium tracking-widest text-ink-soft uppercase ${className}`}
    >
      {children}
    </span>
  );
}

/// The console's one action shape. `primary` marks the view's main verb (save, send, run) as a
/// filled block of sumi ink — the one weighted element on an otherwise hairline page — warming to
/// clay on hover; everything else stays a quiet outline. One component so every view's buttons
/// agree, instead of each re-deriving the class string.
export function Button({
  primary = false,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { primary?: boolean }) {
  return (
    <button
      {...rest}
      className={
        "rounded-xs border px-4 py-2 font-mono text-xs transition-colors disabled:opacity-45 " +
        (primary
          ? "border-ink bg-ink text-paper enabled:hover:border-clay enabled:hover:bg-clay "
          : "border-line-strong text-ink enabled:hover:border-clay enabled:hover:text-clay ") +
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
        "w-full rounded-xs border border-line bg-paper-raised px-2.5 py-1.5 font-mono text-xs text-ink placeholder:text-ink-faint/70 focus:border-ink-faint focus:outline-none " +
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
        "w-full rounded-xs border border-line bg-paper-raised px-2.5 py-2 text-sm text-ink focus:border-ink-faint focus:outline-none " +
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
        "max-h-72 overflow-auto border-l border-line bg-oat/40 px-3 py-2 font-mono text-xs/relaxed whitespace-pre-wrap text-ink-soft " +
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

/// A row of sibling choices where exactly one is active — the sub-nav inside a view (settings
/// sections, prompt templates). Quieter than the workspace tab bar: the active choice is inked and
/// underlined in clay, the rest recede.
export function Segmented({
  options,
  value,
  onChange,
  className = "",
}: {
  options: ReadonlyArray<{ id: string; label: string }>;
  value: string;
  onChange: (id: string) => void;
  className?: string;
}) {
  return (
    <div className={`flex flex-wrap gap-x-5 gap-y-2 text-sm ${className}`}>
      {options.map((option) => (
        <button
          key={option.id}
          onClick={() => onChange(option.id)}
          className={
            "border-b-2 pb-0.5 transition-colors " +
            (option.id === value
              ? "border-clay font-medium text-ink"
              : "border-transparent text-ink-soft hover:text-ink")
          }
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/// A disclosure toggle — the `▸ label` / `▾ label` affordance used for briefs, deliberations,
/// prompts, and the API reference. One component so the arrow, spacing, and hover behavior agree
/// everywhere. `summary` is the always-visible row beside the label (a count, a duration).
/// The console's established in-flight cue: the sage working pulse. `className` positions the
/// wrapper (e.g. `self-center`, `ml-2`).
export function WorkingPulse({ className = "" }: { className?: string }) {
  return (
    <span className={"relative flex size-1 " + className}>
      <span className="absolute inline-flex size-full animate-ping rounded-full bg-sage opacity-70" />
      <span className="relative inline-flex size-1 rounded-full bg-sage" />
    </span>
  );
}

export function Disclosure({
  open,
  onToggle,
  label,
  summary,
  onSummaryClick,
  summaryTitle,
  className = "",
}: {
  open: boolean;
  onToggle: () => void;
  label: ReactNode;
  summary?: ReactNode;
  /// When set, the summary becomes its own button — a distinct action from toggling — so a row can
  /// both disclose and offer a click target (e.g. an event's seq that moves the timeline cursor). The
  /// summary then carries the clay hover to read as interactive.
  onSummaryClick?: () => void;
  summaryTitle?: string;
  className?: string;
}) {
  // An interactive summary is a sibling button (a button cannot nest in the toggle button), so the row
  // becomes a flex container carrying the shared font and colour; the summary reads as interactive
  // through the clay hover. A plain summary rides inside the toggle button, unchanged.
  if (onSummaryClick !== undefined && summary !== undefined) {
    return (
      <div className={"flex items-baseline gap-2 font-mono text-xs text-ink-soft " + className}>
        <button
          onClick={onToggle}
          className="flex items-baseline gap-2 text-left transition-colors hover:text-ink"
        >
          <span aria-hidden className="inline-block w-3 shrink-0 text-center">
            {open ? "▾" : "▸"}
          </span>
          <span>{label}</span>
        </button>
        <button
          onClick={onSummaryClick}
          title={summaryTitle}
          className="text-ink-faint/70 transition-colors hover:text-clay"
        >
          {summary}
        </button>
      </div>
    );
  }
  return (
    <button
      onClick={onToggle}
      className={
        "flex items-baseline gap-2 text-left font-mono text-xs text-ink-soft transition-colors hover:text-ink " +
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
      className={`flex cursor-pointer items-center gap-2 font-mono text-xs text-ink-soft ${className}`}
    >
      {input}
      {label}
    </label>
  );
}
