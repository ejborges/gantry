// A workflow as the visual editor handles it: stages, and single moves between them.
// The server stores a move that starts from several stages as one transition; here each
// starting stage is its own line on the canvas, and they are merged again when saving.
import dagre from "@dagrejs/dagre";
import type { Status, Transition } from "../api";

export interface WorkflowDef {
  initial: string;
  statuses: Status[];
  transitions: Transition[];
  layout?: Record<string, { x: number; y: number }> | null;
}
export interface Move {
  id: string;
  name: string;
  /** A stage key, or `*` for a move allowed from every stage. */
  from: string;
  to: string;
  requires: string[];
}
export const ANY = "*";
export const CATEGORIES: { key: Status["category"]; label: string; hint: string }[] = [
  { key: "todo", label: "Not started", hint: "Work that is waiting" },
  { key: "in_progress", label: "In progress", hint: "Work someone is doing" },
  { key: "done", label: "Done", hint: "Work that is finished or closed" },
];

export function toMoves(transitions: Transition[]): Move[] {
  return transitions.flatMap((t, i) => t.from.map((from, j) => ({ id: `m${i}_${j}`, name: t.name, from, to: t.to, requires: [...(t.requires ?? [])] })));
}

/** Merge moves with the same name, destination and requirements back into one transition. */
export function fromMoves(moves: Move[]): Transition[] {
  const out: Transition[] = [];
  for (const m of moves) {
    const req = [...m.requires].sort();
    const same = out.find((t) => t.name === m.name && t.to === m.to && JSON.stringify([...(t.requires ?? [])].sort()) === JSON.stringify(req));
    if (!same) out.push({ name: m.name, from: [m.from], to: m.to, ...(req.length ? { requires: req } : {}) });
    else if (same.from.includes(ANY) || m.from === ANY) same.from = [ANY];
    else if (!same.from.includes(m.from)) same.from.push(m.from);
  }
  return out;
}

/** A key for a new stage: lowercase letters, digits and underscores, starting with a letter. */
export function keyFor(name: string, taken: string[]): string {
  let base = name.toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 40);
  if (!/^[a-z]/.test(base)) base = `s_${base}`.replace(/_$/, "");
  if (base === "s") base = "stage";
  let key = base;
  for (let n = 2; taken.includes(key); n++) key = `${base}_${n}`;
  return key;
}

/** What the server would refuse, in the words a person needs to fix it. Empty when it can be saved. */
export function problems(def: WorkflowDef): string[] {
  const out: string[] = [];
  const keys = def.statuses.map((s) => s.key);
  if (def.statuses.length === 0) return ["Add at least one stage."];
  if (def.statuses.some((s) => !s.name.trim())) out.push("Every stage needs a name.");
  if (new Set(keys).size !== keys.length) out.push("Two stages have the same key.");
  if (!keys.includes(def.initial)) out.push("Choose the stage where new work starts.");
  if (!def.statuses.some((s) => s.category === "done")) out.push("At least one stage must count as done.");
  if (def.statuses.length > 60) out.push("A workflow can have at most 60 stages.");
  if (def.transitions.length > 400) out.push("A workflow can have at most 400 moves.");
  if (def.transitions.some((t) => !t.name.trim())) out.push("Every move needs a name.");
  if (def.transitions.some((t) => !keys.includes(t.to) || t.from.some((f) => f !== ANY && !keys.includes(f)))) out.push("A move leads to or from a stage that no longer exists.");
  return out;
}

/** Stages nothing leads to (other than the start): work can never arrive there. */
export function unreachable(def: WorkflowDef): Status[] {
  const from = new Map<string, string[]>();
  for (const t of def.transitions) for (const f of t.from) for (const s of f === ANY ? def.statuses.map((x) => x.key) : [f]) from.set(s, [...(from.get(s) ?? []), t.to]);
  const seen = new Set([def.initial]);
  const queue = [def.initial];
  while (queue.length) for (const next of from.get(queue.shift()!) ?? []) if (!seen.has(next)) (seen.add(next), queue.push(next));
  return def.statuses.filter((s) => !seen.has(s.key));
}

/** Stages work can never leave, other than the finished ones. */
export function deadEnds(def: WorkflowDef): Status[] {
  const any = def.transitions.some((t) => t.from.includes(ANY));
  return def.statuses.filter((s) => s.category !== "done" && !def.transitions.some((t) => t.to !== s.key && (t.from.includes(s.key) || (any && t.from.includes(ANY)))));
}

export const NODE_W = 170, NODE_H = 54;

/** Places for the stages, left to right in the order work flows. Saved places win. */
export function arrange(statuses: Status[], moves: Move[], saved?: Record<string, { x: number; y: number }> | null): Record<string, { x: number; y: number }> {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: 30, ranksep: 56, marginx: 16, marginy: 16 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const s of statuses) g.setNode(s.key, { width: NODE_W, height: NODE_H });
  // Moves backward (reopen, send back) would pull the layout into a knot; lay out by the
  // forward ones only, taking the order of the stages as the direction of flow.
  const at = new Map(statuses.map((s, i) => [s.key, i]));
  for (const m of moves) if (m.from !== ANY && at.has(m.from) && at.has(m.to) && at.get(m.from)! < at.get(m.to)!) g.setEdge(m.from, m.to);
  // Stages that no forward move connects still follow one another in the order they are
  // listed, so a workflow whose moves all start "from any" reads left to right too.
  const linked = new Set(g.edges().flatMap((e) => [e.v, e.w]));
  statuses.forEach((s, i) => {
    const next = statuses[i + 1];
    if (next && !(linked.has(s.key) && linked.has(next.key)) && !g.hasEdge(s.key, next.key)) g.setEdge(s.key, next.key);
  });
  dagre.layout(g);
  const out: Record<string, { x: number; y: number }> = {};
  for (const s of statuses) {
    const p = saved?.[s.key];
    const n = g.node(s.key);
    out[s.key] = p && Number.isFinite(p.x) && Number.isFinite(p.y) ? { x: p.x, y: p.y } : { x: Math.round(n.x - NODE_W / 2), y: Math.round(n.y - NODE_H / 2) };
  }
  return out;
}

/** Stages of the old workflow that work may be sitting in and the new one lacks. */
export function removedStages(before: WorkflowDef, after: WorkflowDef): Status[] {
  return before.statuses.filter((s) => !after.statuses.some((n) => n.key === s.key));
}
