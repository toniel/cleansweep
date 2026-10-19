/** Shared formatting helpers. */

export function formatBytes(n: number | null | undefined): string {
  if (n == null || n < 0) return "—";
  if (n === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

export function formatDate(msOrSec: number, fromSeconds = false): string {
  if (!msOrSec || msOrSec < 0) return "—";
  const ms = fromSeconds ? msOrSec * 1000 : msOrSec;
  return new Date(ms).toLocaleString();
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

export function shortId(id: string, len = 12): string {
  return id.replace(/^sha256:/, "").slice(0, len);
}
