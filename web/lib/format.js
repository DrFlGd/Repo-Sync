// Sizes and dates as the interface shows them.

export function bytes(n) {
  if (n == null || isNaN(n)) return "";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

const parse = (iso) => (iso ? new Date(iso) : null);

/** "3 days ago", "in 2 hours", "just now". */
export function ago(iso) {
  const d = parse(iso);
  if (!d || isNaN(d)) return "";
  const s = Math.round((Date.now() - d.getTime()) / 1000);
  const future = s < 0;
  const a = Math.abs(s);
  const say = (n, unit) => { const t = `${n} ${unit}${n === 1 ? "" : "s"}`; return future ? `in ${t}` : `${t} ago`; };
  if (a < 45) return future ? "in a moment" : "just now";
  if (a < 3600) return say(Math.max(1, Math.round(a / 60)), "minute");
  if (a < 86400) return say(Math.round(a / 3600), "hour");
  if (a < 86400 * 45) return say(Math.round(a / 86400), "day");
  if (a < 86400 * 365) return say(Math.round(a / (86400 * 30)), "month");
  return say(Math.round(a / (86400 * 365)), "year");
}

/** "29 Oct 2026, 03:25" in the computer's time. */
export function when(iso) {
  const d = parse(iso);
  if (!d || isNaN(d)) return "";
  return d.toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function day(iso) {
  const d = parse(iso);
  if (!d || isNaN(d)) return "";
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

export const plural = (n, one, many = one + "s") => `${n} ${n === 1 ? one : many}`;
export const short = (sha) => (sha || "").slice(0, 7);
