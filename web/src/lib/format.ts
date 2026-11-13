// Formatting helpers. Dates follow the browser's locale.

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

/** "3 hours ago", "in 2 days". */
export function relative(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "";
  const diff = (new Date(iso).getTime() - now) / 1000;
  const abs = Math.abs(diff);
  if (abs < 45) return "just now";
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(Math.round(diff / 86400), "day");
  if (abs < 86400 * 365) return rtf.format(Math.round(diff / (86400 * 30)), "month");
  return rtf.format(Math.round(diff / (86400 * 365)), "year");
}

// Times are stored and sent in UTC. They are shown in the time zone the account chose, or,
// when the account says "auto", in the time zone of the device being used.
let zone: string | undefined;
export function setTimeZone(tz: string | null | undefined) {
  zone = undefined;
  if (!tz || tz === "auto") return;
  try {
    new Intl.DateTimeFormat(undefined, { timeZone: tz });
    zone = tz;
  } catch {
    /* an unknown zone name: fall back to the device's */
  }
}
export function timeZone(): string {
  return zone ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
}

export function dateTime(iso: string | null | undefined): string {
  if (!iso) return "";
  return new Date(iso).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short", timeZone: zone });
}

/** Date and time with the zone named: what a message shows next to its author. */
export function stamp(iso: string | null | undefined): string {
  if (!iso) return "";
  return new Date(iso).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit", timeZoneName: "short", timeZone: zone });
}

/** The calendar day a moment falls on in the reader's time zone, as YYYY-MM-DD. */
export function dayKey(iso: string): string {
  const parts = new Intl.DateTimeFormat("en-CA", { year: "numeric", month: "2-digit", day: "2-digit", timeZone: zone }).formatToParts(new Date(iso));
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "";
  return `${get("year")}-${get("month")}-${get("day")}`;
}

/** A calendar date (YYYY-MM-DD) shown without time-zone drift. */
/** Today's date where the reader is, as YYYY-MM-DD. */
export function today(): string {
  return dayKey(new Date().toISOString());
}

export function day(isoDate: string | null | undefined): string {
  if (!isoDate) return "";
  const [y, m, d] = isoDate.slice(0, 10).split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { dateStyle: "medium" });
}

export function isOverdue(isoDate: string | null | undefined, done: boolean, today = new Date()): boolean {
  if (!isoDate || done) return false;
  const t = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
  return isoDate.slice(0, 10) < t;
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  return (parts[0][0] + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export const PRIORITIES = [
  { value: 4, label: "Critical" },
  { value: 3, label: "High" },
  { value: 2, label: "Medium" },
  { value: 1, label: "Low" },
  { value: 0, label: "None" },
];
export function priorityLabel(p: number): string {
  return PRIORITIES.find((x) => x.value === p)?.label ?? "None";
}

export type SlaState = { state: "ok" | "soon" | "late" | "met"; label: string };

/** How a service-level target stands right now. "soon" means under two hours left. */
export function slaState(due: string | null, breached: boolean, metAt: string | null, now = Date.now()): SlaState | null {
  if (!due) return null;
  if (metAt) return breached ? { state: "late", label: "Missed" } : { state: "met", label: "Met" };
  const left = new Date(due).getTime() - now;
  if (breached || left < 0) return { state: "late", label: `Overdue ${duration(-left)}` };
  return { state: left < 2 * 3600_000 ? "soon" : "ok", label: `${duration(left)} left` };
}

export function duration(ms: number): string {
  const m = Math.max(0, Math.round(ms / 60000));
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} h ${m % 60 ? `${m % 60} min` : ""}`.trim();
  return `${Math.floor(h / 24)} d`;
}

export function titleCase(s: string): string {
  const t = s.replace(/[_.]/g, " ");
  return t.charAt(0).toUpperCase() + t.slice(1);
}

/** Whether the person has asked their device for less motion. */
export function reducedMotion(): boolean {
  return typeof window !== "undefined" && !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
}
