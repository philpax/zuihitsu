/// Small presentation helpers, kept in one place so every view renders numbers the same way.

/// A rate in [0, 1] as a whole percentage: `1.0` → `"100%"`.
export function formatRate(rate: number): string {
  return `${Math.round(rate * 100)}%`;
}

/// A duration in milliseconds, in the largest unit that reads cleanly: `11870` → `"11.9s"`.
export function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return `${minutes}m${seconds.toString().padStart(2, "0")}s`;
}

/// A token count, abbreviated past a thousand: `21959` → `"22.0k"`.
export function formatTokens(n: number): string {
  if (n < 1000) return `${Math.round(n)}`;
  return `${(n / 1000).toFixed(1)}k`;
}

/// Epoch milliseconds as a short, calm date: `"13 Jun 2026"`.
export function formatDate(ms: number): string {
  return new Date(ms).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}
