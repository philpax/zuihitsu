import type { ReactNode } from "react";

/// The small mono, upper-case, wide-tracked label that titles a section or stands as an eyebrow —
/// the quiet structural device used throughout, in place of heavier headings.
export function Eyebrow({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <span className={`font-mono text-2xs uppercase tracking-widest text-ink-faint ${className}`}>
      {children}
    </span>
  );
}

/// A faint middot separating inline metadata.
export function Dot() {
  return <span className="text-ink-faint/45">·</span>;
}
