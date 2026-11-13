// The hierarchy as a graph: programs and initiatives (or whatever the levels are called),
// the projects in them, and the work items in those. Every node shows a name, its status
// and who has it, with a robot mark on AI agents. The graph follows the work as it changes:
// it refreshes whenever something in it moves, and keeps the place and the open branches.
import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { hierarchy, tree, zoom as d3zoom, zoomIdentity, select, type HierarchyPointNode, type ZoomBehavior } from "d3";
import { ChevronDown, ChevronRight, Maximize, Minus, Network, Plus } from "lucide-react";
import { qs, useGet, type Person } from "../api";
import { Avatar, Dialog, Empty, Field, Loaded, Markdown, PersonChip, Priority, StatusBadge } from "../ui";
import { day, relative } from "../lib/format";
import { useLiveRefresh } from "../realtime";
import { cssColor } from "./Gantt";

export interface GraphNode {
  id: string;
  kind: "container" | "project" | "item" | "root";
  level: string;
  color?: string;
  label: string;
  title: string;
  status: string;
  status_name?: string;
  person: Person | null;
  url: string;
  open?: number;
  total?: number;
  priority?: number;
  due_date?: string | null;
  start_date?: string | null;
  target_date?: string | null;
}
interface Tree extends GraphNode {
  children: Tree[];
  hidden: number;
}

const STATUS_WORD: Record<string, string> = { todo: "Not started", in_progress: "In progress", done: "Done", active: "Active", planned: "Planned", on_hold: "On hold" };
/** The three states a ring can show, for work items, projects and the levels above them. */
export const ringOf = (status: string) => (status === "active" ? "in_progress" : status === "planned" || status === "on_hold" ? "todo" : status);
export const statusWord = (n: Pick<GraphNode, "status" | "status_name">) => n.status_name ?? STATUS_WORD[n.status] ?? n.status;
const W = 232, H = 42, GAP_X = 64, GAP_Y = 10;

/** Turn the flat nodes and edges into a tree under one root, leaving out what is folded away. */
export function buildTree(nodes: GraphNode[], edges: { from: string; to: string }[], folded: Set<string>, rootLabel: string): Tree {
  const kids = new Map<string, string[]>();
  const hasParent = new Set<string>();
  const byId = new Map(nodes.map((n) => [n.id, n]));
  for (const e of edges) {
    if (!byId.has(e.from) || !byId.has(e.to) || hasParent.has(e.to)) continue;
    hasParent.add(e.to);
    kids.set(e.from, [...(kids.get(e.from) ?? []), e.to]);
  }
  const order = { container: 0, project: 1, item: 2, root: 0 };
  const seen = new Set<string>();
  const make = (n: GraphNode): Tree => {
    seen.add(n.id);
    const ids = (kids.get(n.id) ?? []).filter((id) => !seen.has(id));
    const all = ids.map((id) => byId.get(id)!).sort((a, b) => order[a.kind] - order[b.kind] || a.label.localeCompare(b.label, undefined, { numeric: true }));
    return folded.has(n.id) ? { ...n, children: [], hidden: all.length } : { ...n, children: all.map(make), hidden: 0 };
  };
  const sorted = [...nodes].sort((a, b) => order[a.kind] - order[b.kind] || a.label.localeCompare(b.label, undefined, { numeric: true }));
  // Nothing may go missing: whatever a loop in the data keeps out of reach of every root is
  // shown at the top as well. Reach is worked out before folding hides anything.
  const reach = new Set<string>();
  const walk = (id: string) => {
    if (reach.has(id)) return;
    reach.add(id);
    for (const k of kids.get(id) ?? []) walk(k);
  };
  const tops = sorted.filter((n) => !hasParent.has(n.id));
  tops.forEach((n) => walk(n.id));
  for (const n of sorted) if (!reach.has(n.id)) (tops.push(n), walk(n.id));
  const children = tops.map(make);
  return { id: "root", kind: "root", level: "", label: rootLabel, title: rootLabel, status: "", person: null, url: "", children, hidden: 0 };
}

function NodeDetails({ node, onClose }: { node: GraphNode; onClose: () => void }) {
  const itemQ = useGet<any>(node.kind === "item" ? `/api/v1/items/${node.label}` : null);
  const boxQ = useGet<any>(node.kind === "container" ? `/api/v1/containers/${node.id.slice(2)}` : null);
  const item = itemQ.data;
  const pct = node.total ? Math.round(((node.total - (node.open ?? 0)) / node.total) * 100) : 0;
  return (
    <Dialog
      title={node.kind === "item" ? `${node.label} · ${node.level}` : `${node.level}: ${node.kind === "project" ? node.title : node.label}`}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Close</button>
          <Link className="btn primary" to={node.url}>
            Open {node.kind === "item" ? node.label : node.level.toLowerCase()}
          </Link>
        </>
      }
    >
      <div className="stack">
        {node.kind === "item" && <h3 style={{ margin: 0 }}>{node.title}</h3>}
        <dl className="facts">
          <dt>Status</dt>
          <dd>
            <StatusBadge name={statusWord(node)} category={ringOf(node.status)} />
          </dd>
          <dt>{node.kind === "item" ? "Assignee" : node.kind === "project" ? "Lead" : "Owner"}</dt>
          <dd>
            <PersonChip person={node.person} fallback={node.kind === "item" ? "Unassigned" : "Nobody"} presence />
          </dd>
          {node.kind === "project" && (
            <>
              <dt>Work</dt>
              <dd>
                {node.total === 0 ? "No work items yet" : `${node.open} open of ${node.total} · ${pct}% finished`}
                {node.total ? <span className="meter" aria-hidden="true"><span style={{ width: `${pct}%` }} /></span> : null}
              </dd>
            </>
          )}
          {node.kind === "item" && (
            <>
              <dt>Priority</dt>
              <dd><Priority value={node.priority ?? 2} /></dd>
              <dt>Due</dt>
              <dd>{node.due_date ? day(node.due_date) : <span className="muted">No due date</span>}</dd>
              {item?.reporter && (
                <>
                  <dt>Reporter</dt>
                  <dd><PersonChip person={item.reporter} /></dd>
                </>
              )}
              {item?.labels?.length > 0 && (
                <>
                  <dt>Labels</dt>
                  <dd>{item.labels.join(", ")}</dd>
                </>
              )}
              {item?.updated_at && (
                <>
                  <dt>Last change</dt>
                  <dd>{relative(item.updated_at)}</dd>
                </>
              )}
            </>
          )}
          {node.kind === "container" && (
            <>
              <dt>Dates</dt>
              <dd>{node.start_date || node.target_date ? `${node.start_date ? day(node.start_date) : "No start"} to ${node.target_date ? day(node.target_date) : "no target"}` : <span className="muted">Not set</span>}</dd>
              {boxQ.data && (
                <>
                  <dt>Holds</dt>
                  <dd>
                    {boxQ.data.children?.length ?? 0} beneath it, {boxQ.data.projects?.length ?? 0} {boxQ.data.projects?.length === 1 ? "project" : "projects"}
                  </dd>
                </>
              )}
            </>
          )}
        </dl>
        {node.kind === "item" && item?.description && <Markdown source={String(item.description).slice(0, 1200)} compact refs={item.description_refs} />}
        {node.kind === "container" && boxQ.data?.description && <Markdown source={String(boxQ.data.description).slice(0, 1200)} compact />}
      </div>
    </Dialog>
  );
}

export default function HierarchyGraph({ project, container, tall }: { project?: string; container?: string; tall?: boolean }) {
  const [done, setDone] = useState(false);
  const [down, setDown] = useState(false);
  const [folded, setFolded] = useState<Set<string> | null>(null);
  const [open, setOpen] = useState<GraphNode | null>(null);
  const [find, setFind] = useState("");
  const q = useGet<{ nodes: GraphNode[]; edges: { from: string; to: string }[]; truncated: boolean }>(`/api/v1/graph/hierarchy${qs({ done, project, container })}`, { placeholderData: (prev: any) => prev });
  const [pulse, setPulse] = useState(0);
  useLiveRefresh(["work", "planning"], () => setPulse((n) => n + 1));
  const view = useRef<HTMLDivElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const zoomer = useRef<ZoomBehavior<HTMLDivElement, unknown> | null>(null);
  const [scale, setScale] = useState(1);

  // A large graph opens with the projects folded, so the first view is the shape of the
  // hierarchy rather than thousands of work items.
  const data = q.data;
  useEffect(() => {
    if (!data || folded) return;
    const items = data.nodes.filter((n) => n.kind === "item").length;
    setFolded(new Set(items > 30 && !project ? data.nodes.filter((n) => n.kind === "project").map((n) => n.id) : []));
  }, [data, folded, project]);

  const layout = useMemo(() => {
    if (!data) return null;
    const root = hierarchy(buildTree(data.nodes, data.edges, folded ?? new Set(), "Workspace"), (d) => d.children);
    const t = tree<Tree>().nodeSize(down ? [W + 18, H + GAP_X] : [H + GAP_Y, W + GAP_X]);
    const laid = t(root);
    const all = laid.descendants().filter((d) => d.data.kind !== "root");
    const pos = (d: HierarchyPointNode<Tree>) => (down ? { x: d.x, y: d.y } : { x: d.y, y: d.x });
    const xs = all.map((d) => pos(d).x), ys = all.map((d) => pos(d).y);
    const minX = Math.min(0, ...xs), minY = Math.min(0, ...ys);
    return {
      nodes: all.map((d) => ({ d, x: pos(d).x - minX, y: pos(d).y - minY })),
      links: laid.links().filter((l) => l.source.data.kind !== "root").map((l) => ({ id: l.target.data.id, a: { x: pos(l.source).x - minX, y: pos(l.source).y - minY }, b: { x: pos(l.target).x - minX, y: pos(l.target).y - minY } })),
      width: Math.max(...xs, 0) - minX + W,
      height: Math.max(...ys, 0) - minY + H,
    };
  }, [data, folded, down]);

  useEffect(() => {
    const el = view.current;
    if (!el) return;
    const z = d3zoom<HTMLDivElement, unknown>()
      .scaleExtent([0.15, 2])
      .filter((e: any) => (e.type === "wheel" ? e.ctrlKey || e.metaKey || !e.target.closest?.(".graph-node") : !e.target.closest?.("button, a, input")))
      .on("zoom", (e) => {
        if (stage.current) stage.current.style.transform = `translate(${e.transform.x}px, ${e.transform.y}px) scale(${e.transform.k})`;
        setScale(e.transform.k);
      });
    zoomer.current = z;
    select(el).call(z).on("dblclick.zoom", null);
    return () => void select(el).on(".zoom", null);
  }, [!!data]);

  function fit(readable = false) {
    const el = view.current;
    if (!el || !layout || !zoomer.current) return;
    const k = Math.min(1, (el.clientWidth - 40) / layout.width, (el.clientHeight - 40) / layout.height);
    // "Fit" shows everything. When the graph first opens it stays readable instead, starting
    // from the top of the hierarchy; the rest is a drag away.
    const s = Math.max(readable ? 0.7 : 0.15, k);
    const x = layout.width * s <= el.clientWidth - 40 ? (el.clientWidth - layout.width * s) / 2 : 20;
    const y = layout.height * s <= el.clientHeight - 40 ? (el.clientHeight - layout.height * s) / 2 : 20;
    select(el).call(zoomer.current.transform, zoomIdentity.translate(x, y).scale(s));
  }
  const fitted = useRef(false);
  useEffect(() => {
    if (layout && folded && !fitted.current && view.current) {
      fitted.current = true;
      fit(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layout, folded]);
  const zoomBy = (f: number) => view.current && zoomer.current && select(view.current).call(zoomer.current.scaleBy, f);
  const toggle = (id: string) => setFolded((f) => (f?.has(id) ? new Set([...f].filter((x) => x !== id)) : new Set(f ?? []).add(id)));
  const term = find.trim().toLowerCase();
  const hit = (n: GraphNode) => term.length > 1 && (n.label.toLowerCase().includes(term) || n.title.toLowerCase().includes(term) || (n.person?.name.toLowerCase().includes(term) ?? false));
  const matches = term.length > 1 && data ? data.nodes.filter(hit).length : 0;

  return (
    <div className="graph">
      <div className="filters">
        <Field label="Find in the graph">
          <input className="input" type="search" value={find} onChange={(e) => setFind(e.target.value)} placeholder="Name, key or person" />
        </Field>
        <label className="check">
          <input type="checkbox" checked={done} onChange={(e) => setDone(e.target.checked)} /> Finished work
        </label>
        <label className="check">
          <input type="checkbox" checked={down} onChange={(e) => (setDown(e.target.checked), (fitted.current = false))} /> Top to bottom
        </label>
        <span className="grow" />
        {term.length > 1 && <span className="small muted" role="status">{matches} {matches === 1 ? "match" : "matches"}</span>}
        <span className="small muted" role="status" key={pulse} title="The graph updates by itself when work changes">
          <span className="live-dot" aria-hidden="true" /> Live
        </span>
        <div className="row" role="group" aria-label="Zoom">
          <button className="btn icon sm" aria-label="Zoom out" onClick={() => zoomBy(1 / 1.3)}><Minus /></button>
          <span className="small num" style={{ minWidth: 42, textAlign: "center" }}>{Math.round(scale * 100)}%</span>
          <button className="btn icon sm" aria-label="Zoom in" onClick={() => zoomBy(1.3)}><Plus /></button>
          <button className="btn icon sm" aria-label="Fit the whole graph" title="Fit the whole graph" onClick={() => fit()}><Maximize /></button>
        </div>
      </div>
      <Loaded q={q} what="Drawing the hierarchy" lines={8}>
        {(g) =>
          g.nodes.length === 0 ? (
            <Empty icon={<Network />} title="Nothing to draw yet">
              <p>The graph shows projects and their work once there are some you can see.</p>
            </Empty>
          ) : (
            <>
              {g.truncated && <div className="notice">Only the 3,000 most recently changed work items are drawn. Open a single program or project to see all of its work.</div>}
              <div className={`graph-view ${tall ? "tall" : ""}`} ref={view} tabIndex={0} role="group" aria-label="Hierarchy graph. Drag to move, hold Ctrl and scroll to zoom. Every node is a button.">
                <div className="graph-stage" ref={stage} style={{ width: layout?.width, height: layout?.height }}>
                  <svg className="graph-links" width={layout?.width} height={layout?.height} aria-hidden="true">
                    {layout?.links.map((l) => {
                      const a = down ? { x: l.a.x + W / 2, y: l.a.y + H } : { x: l.a.x + W, y: l.a.y + H / 2 };
                      const b = down ? { x: l.b.x + W / 2, y: l.b.y } : { x: l.b.x, y: l.b.y + H / 2 };
                      const d = down ? `M${a.x},${a.y} C${a.x},${(a.y + b.y) / 2} ${b.x},${(a.y + b.y) / 2} ${b.x},${b.y}` : `M${a.x},${a.y} C${(a.x + b.x) / 2},${a.y} ${(a.x + b.x) / 2},${b.y} ${b.x},${b.y}`;
                      return <path key={l.id} d={d} />;
                    })}
                  </svg>
                  {layout?.nodes.map(({ d, x, y }) => {
                    const n = d.data;
                    const count = n.children.length + n.hidden;
                    const state = ringOf(n.status);
                    return (
                      <div key={n.id} className={`graph-node ${n.kind} ${state} ${hit(n) ? "hit" : term.length > 1 ? "dim" : ""}`} style={{ left: x, top: y, width: W, height: H }}>
                        <button className="graph-card" onClick={() => setOpen(n)}>
                          <span className={`status-ring ${state}`} title={statusWord(n)} aria-hidden="true" />
                          <span className="graph-text">
                            <span className="graph-label">
                              {n.kind !== "item" && <em style={n.color ? { color: cssColor(n.color) } : undefined}>{n.level}</em>}
                              {n.kind === "item" ? n.label : n.kind === "project" ? n.label : n.title}
                              {n.kind === "project" && n.total ? <span className="muted"> · {n.open}/{n.total} open</span> : null}
                            </span>
                            {n.kind !== "container" && <span className="graph-title">{n.title}</span>}
                            {n.kind === "container" && <span className="graph-title">{statusWord(n)}</span>}
                          </span>
                          {/* For screen readers: the state the ring shows, and whose picture follows. */}
                          <span className="sr-only">
                            . {statusWord(n)}. {n.kind === "item" ? "Assigned to" : n.kind === "project" ? "Led by" : "Owned by"}{" "}
                          </span>
                          <Avatar person={n.person} size="sm" />
                        </button>
                        {count > 0 && (
                          <button className="graph-fold" aria-expanded={n.hidden === 0} aria-label={`${n.hidden ? "Show" : "Hide"} the ${count} beneath ${n.label}`} title={`${n.hidden ? "Show" : "Hide"} the ${count} beneath`} onClick={() => toggle(n.id)}>
                            {n.hidden ? <ChevronRight /> : <ChevronDown />}
                            {n.hidden ? <span className="num">{count}</span> : null}
                          </button>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
              <ul className="graph-legend small muted" aria-label="What the rings mean">
                <li><span className="status-ring todo" aria-hidden="true" /> Not started</li>
                <li><span className="status-ring in_progress" aria-hidden="true" /> In progress</li>
                <li><span className="status-ring done" aria-hidden="true" /> Done</li>
                <li>A robot mark on a picture means the account is an AI agent.</li>
              </ul>
            </>
          )
        }
      </Loaded>
      {open && <NodeDetails node={open} onClose={() => setOpen(null)} />}
    </div>
  );
}
