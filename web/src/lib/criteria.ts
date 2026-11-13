// The criteria search as the page holds it: a list of "field, comparison, value" lines.
// The server checks every field, comparison and value again; this only shapes the form.

export interface FieldInfo {
  key: string;
  kind: string;
  label: string;
  ops: string[];
  custom?: boolean;
  options?: string[];
}
export interface Criterion {
  field: string;
  op: string;
  value?: unknown;
}
export interface ItemQuery {
  match: "all" | "any";
  criteria: Criterion[];
  sort: { field: string; dir: "asc" | "desc" }[];
}
export interface Option {
  value: string | number;
  label: string;
}

export const EMPTY_QUERY: ItemQuery = { match: "all", criteria: [], sort: [{ field: "updated", dir: "desc" }] };

export const OP_LABEL: Record<string, string> = {
  contains: "contains",
  not_contains: "does not contain",
  starts_with: "starts with",
  eq: "is",
  ne: "is not",
  in: "is any of",
  not_in: "is none of",
  all: "has all of",
  gt: "is more than",
  gte: "is at least",
  lt: "is less than",
  lte: "is at most",
  on: "is on",
  before: "is before",
  after: "is after",
  on_or_before: "is on or before",
  on_or_after: "is on or after",
  between: "is between",
  is_empty: "is empty",
  is_not_empty: "is not empty",
  is: "is",
};
export const SORT_LABEL: Record<string, string> = {
  key: "Key", title: "Name", status: "Status", priority: "Priority", type: "Type", project: "Project", assignee: "Assignee", reporter: "Reporter",
  created: "Created", updated: "Updated", resolved: "Resolved", started: "Start date", due: "Due date", estimate: "Estimate", rank: "Board order",
};

/** What kind of control a line needs for its value. */
export type Control = "none" | "text" | "number" | "choice" | "people" | "date" | "dates" | "flag" | "priority";

export function controlFor(f: FieldInfo | undefined, op: string): Control {
  if (!f || op === "is_empty" || op === "is_not_empty" || f.kind === "watcher") return "none";
  if (f.key === "priority") return "priority";
  switch (f.kind) {
    case "number":
      return "number";
    case "choice":
    case "id":
    case "labels":
    case "select":
    case "multiselect":
      return "choice";
    case "user":
      return "people";
    case "date":
      return op === "between" ? "dates" : "date";
    case "flag":
    case "checkbox":
      return "flag";
    default:
      return "text";
  }
}

/** The value a line starts with when its field or comparison changes. */
export function defaultValue(f: FieldInfo | undefined, op: string): unknown {
  switch (controlFor(f, op)) {
    case "none":
      return f?.kind === "watcher" ? ["me"] : undefined;
    case "choice":
    case "people":
      return [];
    case "flag":
      return true;
    case "priority":
      return 3;
    case "dates":
      return ["", ""];
    case "number":
      return null;
    default:
      return "";
  }
}

export function newCriterion(f: FieldInfo): Criterion {
  const op = f.ops[0];
  return { field: f.key, op, value: defaultValue(f, op) };
}

/** Whether a line says enough to be sent. Unfinished lines are left out of the search. */
export function complete(c: Criterion, f: FieldInfo | undefined): boolean {
  if (!f || !f.ops.includes(c.op)) return false;
  switch (controlFor(f, c.op)) {
    case "none":
    case "flag":
      return true;
    case "choice":
    case "people":
      return Array.isArray(c.value) && c.value.length > 0;
    case "number":
    case "priority":
      return typeof c.value === "number" && Number.isFinite(c.value);
    case "date":
      return isDate(c.value);
    case "dates":
      return Array.isArray(c.value) && c.value.length === 2 && isDate(c.value[0]) && isDate(c.value[1]);
    default:
      return typeof c.value === "string" && c.value.trim().length > 0;
  }
}
const isDate = (v: unknown) => (typeof v === "string" && /^\d{4}-\d{2}-\d{2}$/.test(v)) || (typeof v === "object" && v !== null && Number.isInteger((v as any).days));

/** The search as the server takes it: finished lines only. */
export function forServer(q: ItemQuery, fields: FieldInfo[]): { match: string; criteria: Criterion[]; sort: ItemQuery["sort"] } {
  return {
    match: q.match,
    criteria: q.criteria
      .filter((c) => complete(c, fields.find((f) => f.key === c.field)))
      .map((c) => (controlFor(fields.find((f) => f.key === c.field), c.op) === "none" && c.value === undefined ? { field: c.field, op: c.op } : { field: c.field, op: c.op, value: typeof c.value === "string" ? c.value.trim() : c.value })),
    sort: q.sort.filter((s) => s.field),
  };
}

export const RELATIVE: { days: number; label: string }[] = [
  { days: 0, label: "today" },
  { days: -1, label: "yesterday" },
  { days: -7, label: "7 days ago" },
  { days: -30, label: "30 days ago" },
  { days: -90, label: "90 days ago" },
  { days: 1, label: "tomorrow" },
  { days: 7, label: "in 7 days" },
  { days: 30, label: "in 30 days" },
];
export function dateWords(v: unknown, format: (iso: string) => string): string {
  if (typeof v === "string") return v ? format(v) : "…";
  const days = (v as any)?.days;
  if (!Number.isInteger(days)) return "…";
  return RELATIVE.find((r) => r.days === days)?.label ?? (days < 0 ? `${-days} days ago` : `in ${days} days`);
}

/** One line in plain words, for the summary of a saved search and for screen readers. */
export function describe(c: Criterion, f: FieldInfo | undefined, label: (field: string, value: unknown) => string, format: (iso: string) => string): string {
  if (!f) return c.field;
  const op = OP_LABEL[c.op] ?? c.op;
  switch (controlFor(f, c.op)) {
    case "none":
      return f.kind === "watcher" ? "I follow it" : `${f.label} ${op}`;
    case "flag":
      return c.value === false ? `Not: ${f.label.toLowerCase()}` : f.label;
    case "choice":
    case "people":
      return `${f.label} ${op} ${(Array.isArray(c.value) ? c.value : []).map((v) => label(f.key, v)).join(", ") || "…"}`;
    case "priority":
      return `${f.label} ${op} ${label(f.key, c.value)}`;
    case "date":
      return `${f.label} ${op} ${dateWords(c.value, format)}`;
    case "dates":
      return `${f.label} ${op} ${dateWords((c.value as any)?.[0], format)} and ${dateWords((c.value as any)?.[1], format)}`;
    default:
      return `${f.label} ${op} ${c.value === null || c.value === undefined || c.value === "" ? "…" : String(c.value)}`;
  }
}

/** The search in an address, so a result list can be bookmarked and shared. */
export function encodeQuery(q: ItemQuery): string {
  return JSON.stringify({ m: q.match === "any" ? "any" : undefined, c: q.criteria.map((c) => [c.field, c.op, c.value]), s: q.sort.map((s) => [s.field, s.dir]) });
}
export function decodeQuery(text: string | null): ItemQuery | null {
  if (!text) return null;
  try {
    const raw = JSON.parse(text);
    if (typeof raw !== "object" || raw === null || !Array.isArray(raw.c)) return null;
    const criteria = raw.c.filter((c: unknown) => Array.isArray(c) && typeof c[0] === "string" && typeof c[1] === "string").slice(0, 30).map((c: any[]) => ({ field: c[0], op: c[1], value: c[2] }));
    const sort = (Array.isArray(raw.s) ? raw.s : []).filter((s: unknown) => Array.isArray(s) && typeof s[0] === "string").slice(0, 4).map((s: any[]) => ({ field: s[0], dir: s[1] === "desc" ? "desc" : "asc" }));
    return { match: raw.m === "any" ? "any" : "all", criteria, sort };
  } catch {
    return null;
  }
}

/** A saved search as stored by the server, back into the page's shape. */
export function fromSaved(query: any): ItemQuery {
  return {
    match: query?.match === "any" ? "any" : "all",
    criteria: Array.isArray(query?.criteria) ? query.criteria.map((c: any) => ({ field: String(c.field), op: String(c.op), value: c.value })) : [],
    sort: Array.isArray(query?.sort) ? query.sort.map((s: any) => ({ field: String(s.field), dir: s.dir === "desc" ? "desc" : "asc" })) : [],
  };
}
