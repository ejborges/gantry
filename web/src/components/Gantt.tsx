// A Gantt chart that is edited in place. Dragging a bar, or pulling one of its ends, changes
// the work item's start and due dates through the ordinary API, with the same permissions
// and the same history as editing the item. The chart's owner adds visual cues on top:
// marker lines, shaded spans of dates, and colored bands around rows that belong together.
import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { CalendarPlus, Plus, Trash2 } from "lucide-react";
import type { Person } from "../api";
import { Avatar, Field } from "../ui";
import { day, today } from "../lib/format";
import { colorFor, type Peer } from "../realtime";

export interface GanttItem {
  id: string;
  key: string;
  project: string;
  title: string;
  type: string;
  type_name: string;
  type_color: string;
  status_category: string;
  status_name: string;
  priority: number;
  start_date: string | null;
  due_date: string | null;
  parent_id: string | null;
  assignee: Person | null;
}
export interface Annotation {
  id: string;
  type: "line" | "range" | "group";
  color: string;
  label?: string;
  date?: string;
  style?: "solid" | "dashed" | "dotted";
  start?: string;
  end?: string;
  items?: string[];
}
export interface GanttSettings {
  zoom?: "day" | "week" | "month" | "quarter";
  group_by?: "none" | "project" | "assignee" | "type" | "parent" | "status";
  color_by?: "status" | "type" | "priority" | "assignee";
  show_done?: boolean;
  show_undated?: boolean;
  show_cycles?: boolean;
  show_today?: boolean;
}

export const PALETTE = ["blue", "green", "amber", "red", "purple", "teal", "pink", "neutral"];
const VAR: Record<string, string> = { neutral: "var(--muted)", blue: "var(--progress)", green: "var(--done)", amber: "var(--warn-solid)", red: "var(--danger)", purple: "var(--purple)", teal: "var(--teal)", pink: "var(--viz-5)" };
export const cssColor = (c: string | undefined) => (c && c.startsWith("#") ? c : (VAR[c ?? "neutral"] ?? VAR.neutral));
const PX = { day: 34, week: 14, month: 5, quarter: 2.2 };
const ROW = 34;
const MS = 86_400_000;

// Dates are handled as whole days in UTC so that no time zone can shift a bar by a day.
const toDay = (iso: string) => Math.round(Date.parse(`${iso}T00:00:00Z`) / MS);
const fromDay = (n: number) => new Date(n * MS).toISOString().slice(0, 10);
export const addDays = (iso: string, n: number) => fromDay(toDay(iso) + n);

interface RowT {
  kind: "group" | "item";
  label?: string;
  item?: GanttItem;
}

function barColor(i: GanttItem, by: GanttSettings["color_by"]): string {
  if (by === "type") return cssColor(i.type_color);
  if (by === "priority") return ["var(--faint)", "var(--todo)", "var(--progress)", "var(--warn-solid)", "var(--danger)"][i.priority] ?? "var(--todo)";
  if (by === "assignee") return i.assignee ? colorFor(i.assignee.id) : "var(--faint)";
  return i.status_category === "done" ? "var(--done)" : i.status_category === "in_progress" ? "var(--progress)" : "var(--todo)";
}

interface Props {
  items: GanttItem[];
  cycles?: { id: string; name: string; project: string; start_date: string | null; end_date: string | null }[];
  editable: Record<string, boolean>;
  settings: GanttSettings;
  annotations: Annotation[];
  /** The chart's owner: may add, change and remove the visual cues. */
  canAnnotate?: boolean;
  onAnnotations?: (a: Annotation[]) => void;
  /** Change a work item's dates. Rejects with a message when the server refuses. */
  onDates: (item: GanttItem, start: string | null, due: string | null) => Promise<unknown>;
  /** Others looking at the same chart, to show whose bar is being moved. */
  peers?: Peer[];
  onFocusItem?: (key: string | null) => void;
}

export default function Gantt({ items, cycles = [], editable, settings, annotations, canAnnotate, onAnnotations, onDates, peers = [], onFocusItem }: Props) {
  const zoom = settings.zoom ?? "week";
  const px = PX[zoom];
  const now = today();
  const [drag, setDrag] = useState<{ key: string; mode: "move" | "start" | "end"; x: number; delta: number } | null>(null);
  const [pending, setPending] = useState<Record<string, { start: string | null; due: string | null }>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [panel, setPanel] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);

  const shown = useMemo(() => items.filter((i) => (settings.show_done !== false || i.status_category !== "done") && (settings.show_undated !== false || i.start_date || i.due_date)), [items, settings.show_done, settings.show_undated]);
  const rows = useMemo<RowT[]>(() => {
    const g = settings.group_by ?? "none";
    if (g === "none") return shown.map((item) => ({ kind: "item", item }));
    const byId = new Map(items.map((i) => [i.id, i]));
    const name = (i: GanttItem) => (g === "project" ? i.project : g === "assignee" ? (i.assignee ? `${i.assignee.name}${i.assignee.kind === "agent" ? " (AI agent)" : ""}` : "Unassigned") : g === "type" ? i.type_name : g === "status" ? i.status_name : i.parent_id ? `${byId.get(i.parent_id)?.key ?? ""} ${byId.get(i.parent_id)?.title ?? "Parent"}`.trim() : "No parent");
    const groups = new Map<string, GanttItem[]>();
    for (const i of shown) groups.set(name(i), [...(groups.get(name(i)) ?? []), i]);
    return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b)).flatMap(([label, list]) => [{ kind: "group" as const, label: `${label} (${list.length})` }, ...list.map((item) => ({ kind: "item" as const, item }))]);
  }, [shown, items, settings.group_by]);

  // The span of dates shown: everything dated, the cues, and some room around today.
  const [first, last] = useMemo(() => {
    const days = [toDay(now) - 10, toDay(now) + 30];
    for (const i of items) for (const d of [i.start_date, i.due_date]) if (d) days.push(toDay(d));
    for (const a of annotations) for (const d of [a.date, a.start, a.end]) if (d) days.push(toDay(d));
    const lo = Math.min(...days) - 7, hi = Math.max(...days) + 14;
    // Start on the first of a month so the header reads cleanly.
    const s = new Date(lo * MS);
    return [Math.round(Date.UTC(s.getUTCFullYear(), s.getUTCMonth(), 1) / MS), Math.min(hi, lo + 3660)];
  }, [items, annotations, now]);
  const width = (last - first + 1) * px;
  const x = (iso: string) => (toDay(iso) - first) * px;

  const months = useMemo(() => {
    const out: { label: string; left: number; width: number }[] = [];
    let d = new Date(first * MS);
    while (Math.round(d.getTime() / MS) <= last) {
      const startDay = Math.round(d.getTime() / MS);
      const next = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1));
      const endDay = Math.min(last + 1, Math.round(next.getTime() / MS));
      out.push({ label: d.toLocaleDateString(undefined, { month: zoom === "quarter" || zoom === "month" ? "short" : "long", year: "numeric", timeZone: "UTC" }), left: (startDay - first) * px, width: (endDay - startDay) * px });
      d = next;
    }
    return out;
  }, [first, last, px, zoom]);
  const ticks = useMemo(() => {
    if (zoom === "month" || zoom === "quarter") return [];
    const out: { left: number; label: string; weekend: boolean }[] = [];
    for (let n = first; n <= last; n++) {
      const d = new Date(n * MS);
      const dow = d.getUTCDay();
      if (zoom === "day" || dow === 1) out.push({ left: (n - first) * px, label: String(d.getUTCDate()), weekend: dow === 0 || dow === 6 });
    }
    return out;
  }, [first, last, px, zoom]);

  useEffect(() => {
    // Open on today.
    const el = scroller.current;
    if (el) el.scrollLeft = Math.max(0, x(now) - el.clientWidth / 3);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [zoom, first]);

  const datesOf = (i: GanttItem) => pending[i.key] ?? { start: i.start_date, due: i.due_date };
  function moved(i: GanttItem, mode: "move" | "start" | "end", delta: number) {
    const d = datesOf(i);
    const start = d.start ?? d.due!, due = d.due ?? d.start!;
    let ns = mode === "end" ? start : addDays(start, delta);
    let nd = mode === "start" ? due : addDays(due, delta);
    if (ns > nd) mode === "start" ? (ns = nd) : (nd = ns);
    // A bar that has only one date keeps having only that date unless an end is pulled.
    return { start: d.start || mode !== "move" ? ns : null, due: d.due || mode !== "move" ? nd : null };
  }
  async function commit(i: GanttItem, next: { start: string | null; due: string | null }) {
    const was = datesOf(i);
    if (next.start === was.start && next.due === was.due) return;
    setPending((p) => ({ ...p, [i.key]: next }));
    try {
      await onDates(i, next.start, next.due);
    } finally {
      // The saved dates arrive with the refreshed data; until then the bar stays where it was put.
      setTimeout(() => setPending((p) => Object.fromEntries(Object.entries(p).filter(([k]) => k !== i.key))), 1200);
    }
  }

  function startDrag(e: React.PointerEvent, i: GanttItem, mode: "move" | "start" | "end") {
    if (!editable[i.project] || e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setDrag({ key: i.key, mode, x: e.clientX, delta: 0 });
    onFocusItem?.(i.key);
  }
  function onMove(e: React.PointerEvent) {
    if (drag) setDrag({ ...drag, delta: Math.round((e.clientX - drag.x) / px) });
  }
  function endDrag(i: GanttItem) {
    if (!drag) return;
    const d = drag;
    setDrag(null);
    onFocusItem?.(null);
    if (d.delta !== 0) void commit(i, moved(i, d.mode, d.delta));
  }
  function onKey(e: React.KeyboardEvent, i: GanttItem) {
    if (!editable[i.project] || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    const delta = (e.key === "ArrowLeft" ? -1 : 1) * (e.ctrlKey || e.metaKey ? 7 : 1);
    void commit(i, moved(i, e.shiftKey ? "end" : e.altKey ? "start" : "move", delta));
  }

  const groupOf = (key: string) => annotations.filter((a) => a.type === "group" && a.items?.includes(key));
  const lines = annotations.filter((a) => a.type === "line" && a.date);
  const ranges = annotations.filter((a) => a.type === "range" && a.start && a.end);
  const groups = annotations.filter((a) => a.type === "group");
  const change = (id: string, patch: Partial<Annotation>) => onAnnotations?.(annotations.map((a) => (a.id === id ? { ...a, ...patch } : a)));
  const add = (a: Omit<Annotation, "id">) => onAnnotations?.([...annotations, { ...a, id: `a${Date.now().toString(36)}${Math.random().toString(36).slice(2, 5)}` } as Annotation]);
  const movers = new Map(peers.filter((p) => typeof p.state.focus === "string" && p.state.focus.startsWith("bar:")).map((p) => [String(p.state.focus).slice(4), p.user]));
  // Beneath the rows there is a band for the names of cycles and shaded spans.
  const band = (settings.show_cycles !== false && cycles.some((c) => c.start_date && c.end_date)) || annotations.some((a) => a.type === "range") ? 40 : 0;
  const height = rows.length * ROW + band;

  return (
    <div className="gantt-wrap">
      {canAnnotate && (
        <div className="row wrap" style={{ marginBottom: 8 }}>
          <button className="btn sm" aria-expanded={panel} onClick={() => setPanel(!panel)}>
            Lines, spans and groups <span className="count num">{annotations.length}</span>
          </button>
          {panel && (
            <>
              <button className="btn sm" onClick={() => add({ type: "line", date: now, color: "red", label: "Deadline", style: "dashed" })}>
                <Plus /> Marker line
              </button>
              <button className="btn sm" onClick={() => add({ type: "range", start: now, end: addDays(now, 13), color: "amber", label: "Shaded span" })}>
                <Plus /> Shaded span
              </button>
              <button className="btn sm" disabled={selected.size === 0} title={selected.size === 0 ? "Select the rows that belong together first" : undefined} onClick={() => (add({ type: "group", items: [...selected], color: "blue", label: "Group" }), setSelected(new Set()))}>
                <Plus /> Group the {selected.size || ""} selected {selected.size === 1 ? "row" : "rows"}
              </button>
            </>
          )}
        </div>
      )}
      {canAnnotate && panel && annotations.length > 0 && (
        <div className="panel gantt-cues">
          {annotations.map((a) => (
            <div key={a.id} className="list-row wrap">
              <span className="badge outline">{a.type === "line" ? "Line" : a.type === "range" ? "Span" : "Group"}</span>
              <input className="input" style={{ maxWidth: 200 }} value={a.label ?? ""} maxLength={120} aria-label="Label" placeholder="Label" onChange={(e) => change(a.id, { label: e.target.value })} />
              {a.type === "line" && (
                <>
                  <input className="input" style={{ maxWidth: 160 }} type="date" value={a.date} aria-label="Date" onChange={(e) => e.target.value && change(a.id, { date: e.target.value })} />
                  <select className="select" style={{ maxWidth: 110 }} value={a.style ?? "solid"} aria-label="Line style" onChange={(e) => change(a.id, { style: e.target.value as Annotation["style"] })}>
                    <option value="solid">Solid</option>
                    <option value="dashed">Dashed</option>
                    <option value="dotted">Dotted</option>
                  </select>
                </>
              )}
              {a.type === "range" && (
                <>
                  <input className="input" style={{ maxWidth: 160 }} type="date" value={a.start} max={a.end} aria-label="From" onChange={(e) => e.target.value && change(a.id, { start: e.target.value })} />
                  <input className="input" style={{ maxWidth: 160 }} type="date" value={a.end} min={a.start} aria-label="To" onChange={(e) => e.target.value && change(a.id, { end: e.target.value })} />
                </>
              )}
              {a.type === "group" && <span className="small muted">{a.items?.join(", ")}</span>}
              <span className="row" role="radiogroup" aria-label="Color" style={{ gap: 3 }}>
                {PALETTE.map((c) => (
                  <button key={c} type="button" role="radio" aria-checked={a.color === c} aria-label={c} title={c} className="swatch-btn" style={{ background: cssColor(c) }} onClick={() => change(a.id, { color: c })} />
                ))}
                <input type="color" aria-label="Another color" value={a.color.startsWith("#") ? a.color : "#888888"} onChange={(e) => change(a.id, { color: e.target.value })} />
              </span>
              <span className="grow" />
              <button className="btn ghost icon sm" aria-label={`Remove ${a.label || "this"}`} onClick={() => onAnnotations?.(annotations.filter((x) => x.id !== a.id))}>
                <Trash2 />
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="gantt" style={{ ["--row" as any]: `${ROW}px` }}>
        <div className="gantt-names" role="list" aria-label="Work items on the chart">
          <div className="gantt-head" role="presentation">Work item</div>
          {rows.map((r, n) =>
            r.kind === "group" ? (
              <div key={`g${n}`} className="gantt-name group" role="listitem">{r.label}</div>
            ) : (
              <div key={r.item!.key} className="gantt-name" role="listitem" style={groupOf(r.item!.key)[0] ? { boxShadow: `inset 3px 0 0 ${cssColor(groupOf(r.item!.key)[0].color)}` } : undefined}>
                {canAnnotate && panel && (
                  <input type="checkbox" aria-label={`Select ${r.item!.key} for a group`} checked={selected.has(r.item!.key)} onChange={(e) => setSelected((s) => (e.target.checked ? new Set(s).add(r.item!.key) : new Set([...s].filter((k) => k !== r.item!.key))))} />
                )}
                <Link to={`/i/${r.item!.key}`} className="key">{r.item!.key}</Link>
                <span className={`truncate grow ${r.item!.status_category === "done" ? "muted" : ""}`} title={r.item!.title}>{r.item!.title}</span>
                <Avatar person={r.item!.assignee} size="sm" />
              </div>
            ),
          )}
          {band > 0 && <div className="gantt-foot" role="presentation" />}
        </div>
        <div className="gantt-scroll" ref={scroller} tabIndex={0} role="region" aria-label="Timeline. Scroll sideways to move through time.">
          <div className="gantt-time" style={{ width }}>
            <div className="gantt-head">
              <div className="gantt-months">
                {months.map((m) => (
                  <span key={m.left} style={{ left: m.left, width: m.width }}>{m.width > 46 ? m.label : ""}</span>
                ))}
              </div>
              <div className="gantt-ticks">
                {ticks.map((t) => (
                  <span key={t.left} className={t.weekend ? "weekend" : ""} style={{ left: t.left, width: zoom === "day" ? px : px * 7 }}>{t.label}</span>
                ))}
              </div>
            </div>
            <div className="gantt-body" style={{ height }}>
              {band > 0 && <span className="gantt-foot" />}
              {zoom === "day" && ticks.filter((t) => t.weekend).map((t) => <span key={t.left} className="gantt-weekend" style={{ left: t.left, width: px }} />)}
              {months.map((m) => (
                <span key={m.left} className="gantt-month-line" style={{ left: m.left }} />
              ))}
              {settings.show_cycles !== false &&
                cycles.filter((c) => c.start_date && c.end_date).map((c) => (
                  <span key={c.id} className="gantt-cycle" style={{ left: x(c.start_date!), width: (toDay(c.end_date!) - toDay(c.start_date!) + 1) * px }} title={`${c.project} · ${c.name}: ${day(c.start_date)} to ${day(c.end_date)}`}>
                    <em>{c.name}</em>
                  </span>
                ))}
              {ranges.map((a) => (
                <span key={a.id} className="gantt-range" style={{ left: x(a.start!), width: (toDay(a.end!) - toDay(a.start!) + 1) * px, background: `color-mix(in srgb, ${cssColor(a.color)} 16%, transparent)`, borderColor: cssColor(a.color) }} title={`${a.label ?? "Span"}: ${day(a.start)} to ${day(a.end)}`}>
                  {a.label && <em style={{ color: cssColor(a.color) }}>{a.label}</em>}
                </span>
              ))}
              {rows.map((r, n) => {
                if (r.kind === "group") return <div key={`g${n}`} className="gantt-row group" style={{ top: n * ROW }} />;
                const i = r.item!;
                const base = datesOf(i);
                const d = drag?.key === i.key ? moved(i, drag.mode, drag.delta) : base;
                const s = d.start ?? d.due, e = d.due ?? d.start;
                const band = groupOf(i.key)[0];
                const can = !!editable[i.project];
                const mover = movers.get(i.key);
                return (
                  <div key={i.key} className="gantt-row" style={{ top: n * ROW, background: band ? `color-mix(in srgb, ${cssColor(band.color)} 10%, transparent)` : undefined }}>
                    {band && n > 0 && groupOf(rows[n - 1].item?.key ?? "")[0]?.id !== band.id && band.label && <span className="gantt-band-label" style={{ color: cssColor(band.color) }}>{band.label}</span>}
                    {s && e ? (
                      <div
                        className={`gantt-bar ${can ? "can" : ""} ${drag?.key === i.key ? "dragging" : ""} ${d.start && d.due ? "" : "single"} ${i.status_category === "done" ? "done" : ""}`}
                        style={{ left: x(s), width: Math.max(px, (toDay(e) - toDay(s) + 1) * px), background: barColor(i, settings.color_by), outline: mover ? `2px solid ${colorFor(mover.id)}` : undefined }}
                        role="button"
                        tabIndex={0}
                        aria-label={`${i.key} ${i.title}: ${d.start ? day(d.start) : "no start date"} to ${d.due ? day(d.due) : "no due date"}, ${i.status_name}${can ? ". Arrow keys move it by a day; with Shift they move the due date, with Alt the start date." : ""}`}
                        title={`${i.key} ${i.title}\n${d.start ? day(d.start) : "no start"} → ${d.due ? day(d.due) : "no due date"}${d.start && d.due ? ` (${toDay(d.due) - toDay(d.start) + 1} days)` : ""}${mover ? `\n${mover.name} is moving this` : ""}`}
                        onPointerDown={(ev) => startDrag(ev, i, "move")}
                        onPointerMove={onMove}
                        onPointerUp={() => endDrag(i)}
                        onPointerCancel={() => setDrag(null)}
                        onKeyDown={(ev) => onKey(ev, i)}
                        onFocus={() => onFocusItem?.(i.key)}
                        onBlur={() => onFocusItem?.(null)}
                      >
                        {can && <span className="grip start" onPointerDown={(ev) => (ev.stopPropagation(), startDrag(ev, i, "start"))} onPointerMove={onMove} onPointerUp={() => endDrag(i)} aria-hidden="true" />}
                        <span className="gantt-bar-label">{i.title}</span>
                        {can && <span className="grip end" onPointerDown={(ev) => (ev.stopPropagation(), startDrag(ev, i, "end"))} onPointerMove={onMove} onPointerUp={() => endDrag(i)} aria-hidden="true" />}
                      </div>
                    ) : (
                      can && (
                        <button className="btn ghost sm gantt-set" style={{ left: x(now) + 6 }} onClick={() => void commit(i, { start: now, due: addDays(now, 4) })}>
                          <CalendarPlus /> Put on the chart
                        </button>
                      )
                    )}
                  </div>
                );
              })}
              {settings.show_today !== false && <span className="gantt-today" style={{ left: x(now) + px / 2 }} title={`Today, ${day(now)}`} />}
              {lines.map((a) => (
                <span key={a.id} className="gantt-line" style={{ left: x(a.date!) + px / 2, borderLeftColor: cssColor(a.color), borderLeftStyle: a.style ?? "solid" }} title={`${a.label ?? "Marker"}: ${day(a.date)}`}>
                  {a.label && <em style={{ background: cssColor(a.color) }}>{a.label}</em>}
                </span>
              ))}
            </div>
          </div>
        </div>
      </div>
      {rows.length === 0 && <p className="muted" style={{ padding: 14 }}>No work to show with these choices.</p>}
      {groups.length + lines.length + ranges.length > 0 && (
        <ul className="gantt-legend" aria-label="Lines, spans and groups on this chart">
          {annotations.map((a) => (
            <li key={a.id}>
              <span className="swatch" style={{ background: cssColor(a.color) }} aria-hidden="true" />
              {a.label || (a.type === "line" ? "Marker" : a.type === "range" ? "Span" : "Group")}
              <span className="muted">{a.type === "line" ? ` · ${day(a.date)}` : a.type === "range" ? ` · ${day(a.start)} to ${day(a.end)}` : ` · ${a.items?.length} rows`}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function GanttSettingsBar({ value, onChange }: { value: GanttSettings; onChange: (s: GanttSettings) => void }) {
  const set = (patch: GanttSettings) => onChange({ ...value, ...patch });
  return (
    <div className="filters">
      <Field label="Scale">
        <select className="select" value={value.zoom ?? "week"} onChange={(e) => set({ zoom: e.target.value as GanttSettings["zoom"] })}>
          <option value="day">Days</option>
          <option value="week">Weeks</option>
          <option value="month">Months</option>
          <option value="quarter">Quarters</option>
        </select>
      </Field>
      <Field label="Group rows by">
        <select className="select" value={value.group_by ?? "none"} onChange={(e) => set({ group_by: e.target.value as GanttSettings["group_by"] })}>
          <option value="none">Nothing</option>
          <option value="project">Project</option>
          <option value="assignee">Assignee</option>
          <option value="type">Type</option>
          <option value="status">Status</option>
          <option value="parent">Parent</option>
        </select>
      </Field>
      <Field label="Color bars by">
        <select className="select" value={value.color_by ?? "status"} onChange={(e) => set({ color_by: e.target.value as GanttSettings["color_by"] })}>
          <option value="status">Status</option>
          <option value="type">Type</option>
          <option value="priority">Priority</option>
          <option value="assignee">Assignee</option>
        </select>
      </Field>
      <label className="check">
        <input type="checkbox" checked={value.show_done !== false} onChange={(e) => set({ show_done: e.target.checked })} /> Finished work
      </label>
      <label className="check">
        <input type="checkbox" checked={value.show_undated !== false} onChange={(e) => set({ show_undated: e.target.checked })} /> Work without dates
      </label>
      <label className="check">
        <input type="checkbox" checked={value.show_cycles !== false} onChange={(e) => set({ show_cycles: e.target.checked })} /> Cycles
      </label>
    </div>
  );
}
